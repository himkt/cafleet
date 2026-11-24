package core

import (
	"bytes"
	"context"
	"encoding/json"
	"encoding/json/jsontext"
	"fmt"
	"github.com/ncruces/go-sqlite3"
	"math"
	"path/filepath"
	"reflect"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"testing"
	"time"
)

const (
	ordinaryLiteral = "ordinary fixture λ"
	ordinaryScreen  = "\x1b[2J\x1b[23;1HSynthetic state\r\nState end"
	testStageSettle = 50 * time.Millisecond
)

// awaitWriter yields until the writer has persisted every accepted event or reported a failure,
// within a real-time bound, so a drain observes a settled sink.
func awaitWriter(t *testing.T, writer *eventWriter) {
	t.Helper()
	until := time.Now().Add(10 * time.Second)
	for {
		status := writer.status()
		if status.pendingCount == 0 || status.diagnostic != none[string]() {
			return
		}
		if !time.Now().Before(until) {
			t.Fatalf("the event writer settles: %+v", status)
		}
		runtime.Gosched()
	}
}

func offers(values ...string) [][]byte {
	result := make([][]byte, len(values))
	for index, value := range values {
		result[index] = []byte(value)
	}
	return result
}

// panicTokens is a token source that refuses use, naming purpose in the panic.
type panicTokens struct{ purpose string }

func (p panicTokens) fill(*[32]byte) error {
	panic(p.purpose + " creates no request token")
}

// testInputServices are input services whose token source refuses use, naming purpose.
func testInputServices(catalog *profileCatalog, hooks inputHooks, stageSettle time.Duration, purpose string, c clock) inputServices {
	services := newInputServices(catalog, panicTokens{purpose: purpose}, c)
	services.hooks = hooks
	services.stageSettle = stageSettle
	return services
}

func inputCatalogDefinition() any {
	value := syntheticDefinitions()
	jset(value, fixtureAt(syntheticVersion), "codex", "screens", "synthetic-prompt-plan")
	rule := jget(value, "codex", "profile", "rules", 0)
	jset(rule, "synthetic-composer-empty", "id")
	jset(rule, "composer_empty", "observation")
	jset(rule, "^Synthetic state$", "region", "start_pattern")
	jset(rule, "^State end$", "region", "end_pattern")
	jset(rule, parseTestJSON(`[{"id":"state","scope":"line","pattern":"^Synthetic state$"}]`), "matchers")
	jset(rule, parseTestJSON(`{"operation":null,"choices":[]}`), "extract")
	jset(rule, true, "verified_noninteraction")
	jset(value, parseTestJSON(`{
		"detection":null,"answers":[],"composer":{
			"rule_ids":["synthetic-composer-empty"],"fixture_ids":["`+syntheticFixture+`"],
			"submission":{"prompt":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"","submit_hex":"7e","fixture_ids":["synthetic-prompt-plan"]},"exec":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"2120","submit_hex":"0d","fixture_ids":["synthetic-prompt-plan"]}}
		}
	}`), "codex", "profile", "capabilities")
	addSyntheticComposerText(value, "codex")
	return value
}

func inputCatalog(t testing.TB) *profileCatalog {
	t.Helper()
	return promptPlanCatalog(t, inputCatalogDefinition())
}

// promptPlanCatalog is a catalog built on inputCatalogDefinition's composer whose exec submission
// is a placeholder: only the prompt operation keeps an executable plan.
func promptPlanCatalog(t testing.TB, definition any) *profileCatalog {
	t.Helper()
	return mustLoadCatalog(t, definition).withoutSubmission(codingAgentCodex, composerOperationExec)
}

func encodedLiteral() []byte {
	return append([]byte(ordinaryLiteral), '~')
}

type hostProfile struct {
	backend       codingAgent
	nativeVersion opt[string]
	size          terminalSize
	screen        []byte
	stageSettle   time.Duration
	// clockLead is how far the host clock starts ahead of real time; zero for a test whose storage
	// contention must expire on the real-time database budget.
	clockLead time.Duration
}

func metadataProfile() hostProfile {
	return hostProfile{
		backend:       codingAgentCodex,
		nativeVersion: some(syntheticVersion),
		size:          terminalSize{rows: 24, cols: 80},
		screen:        []byte(ordinaryScreen),
		stageSettle:   testStageSettle,
		clockLead:     runtimeVirtualLead,
	}
}

type hookPoint struct {
	boundary string
	attempt  attemptKey
	count    uint64
	seq      uint64
}

// inputTestHooks records every coordinator checkpoint.
type inputTestHooks struct {
	mu     sync.Mutex
	points []hookPoint
	until  time.Time
}

func (h *inputTestHooks) count(boundary string) int {
	h.mu.Lock()
	defer h.mu.Unlock()
	total := 0
	for _, point := range h.points {
		if point.boundary == boundary {
			total++
		}
	}
	return total
}

func (h *inputTestHooks) reservedAttempts() []attemptKey {
	h.mu.Lock()
	defer h.mu.Unlock()
	var attempts []attemptKey
	for _, point := range h.points {
		if point.boundary == "reserved" {
			attempts = append(attempts, point.attempt)
		}
	}
	return attempts
}

func (h *inputTestHooks) attempt(t *testing.T) attemptKey {
	t.Helper()
	attempts := h.reservedAttempts()
	if len(attempts) == 0 {
		t.Fatal("a real Reserved checkpoint")
	}
	return attempts[0]
}

func (h *inputTestHooks) inputID(t *testing.T) int64 {
	t.Helper()
	receipt, ok := h.attempt(t).(attemptKeyReceipt)
	if !ok {
		t.Fatal("the reserved attempt is a receipt")
	}
	return receipt.id
}

func (h *inputTestHooks) checkpoint(boundary inputBoundary, state inputCheckpoint, deadline time.Time) error {
	if deadline.After(h.until) {
		panic("a checkpoint deadline stays within the fixture deadline")
	}
	var name string
	switch boundary {
	case inputBoundaryReserved:
		name = "reserved"
	case inputBoundaryBeforeWrite:
		name = "before"
	case inputBoundaryAfterWrite:
		name = "after"
	case inputBoundaryTerminalState:
		name = "terminal"
	default:
		panic("invalid inputBoundary")
	}
	h.mu.Lock()
	defer h.mu.Unlock()
	h.points = append(h.points, hookPoint{boundary: name, attempt: state.attempt, count: state.writtenBytes, seq: state.outputSeq})
	return nil
}

type timedStage struct {
	at    time.Time
	bytes []byte
}

// inputPty is the native child's PTY: program holds each completed programmatic input; stages
// holds every separately offered programmatic stage with its offer time, so settle intervals
// between a literal and its submit key are observable.
type inputPty struct {
	t               *testing.T
	child           *fakeInput
	offers          [][]byte
	written         []byte
	program         [][]byte
	stages          []timedStage
	manual          []byte
	short           opt[int]
	failNext        opt[error]
	failure         opt[error]
	expectedProgram []byte
	programOffset   int
	databaseURL     string
	hooks           *inputTestHooks
	clock           *testClock
	// painter paints typed input into the composer, as a native composer shows its unsubmitted
	// literal before the submit key.
	painter composerPainter
	typed   []byte
}

// clearComposer abandons a partly written program: the next programmatic write starts afresh.
func (p *inputPty) clearComposer() {
	p.programOffset = 0
	p.typed = nil
}

func (p *inputPty) echoProgram(data []byte, startsProgram, completed bool) {
	if p.painter == nil {
		return
	}
	if startsProgram || completed {
		p.typed = nil
	}
	if completed {
		return
	}
	p.typed = append(p.typed, data...)
	p.paintComposer()
}

// echoManual echoes typed characters that land in a composer already holding a literal; escape
// sequences such as pointer reports paint nothing.
func (p *inputPty) echoManual(data []byte) {
	if p.painter == nil || len(p.typed) == 0 || data[0] == 0x1b {
		return
	}
	for _, b := range data {
		if b >= 0x20 && b != 0x7f {
			p.typed = append(p.typed, b)
		}
	}
	p.paintComposer()
}

func (p *inputPty) paintComposer() {
	shown := slices.Clone(p.typed)
	for _, marker := range [][]byte{[]byte(pasteOpen), []byte(pasteClose)} {
		shown = bytes.ReplaceAll(shown, marker, nil)
	}
	shown = bytes.TrimPrefix(shown, []byte{0x1b})
	p.child.push(p.painter(string(lossyUTF8(shown))))
}

func (p *inputPty) expectProgram(program []byte) {
	if p.programOffset != 0 {
		panic("no partially written program")
	}
	p.expectedProgram = program
}

func (p *inputPty) stageBytes() [][]byte {
	result := make([][]byte, len(p.stages))
	for index, stage := range p.stages {
		result[index] = stage.bytes
	}
	return result
}

func (p *inputPty) stageGaps() []time.Duration {
	var gaps []time.Duration
	for index := 1; index < len(p.stages); index++ {
		gaps = append(gaps, p.stages[index].at.Sub(p.stages[index-1].at))
	}
	return gaps
}

func (p *inputPty) tryRead(target []byte) (int, error) {
	return p.child.tryRead(target)
}

// assertInProgressReservation asserts a programmatic stage writes only for an in-progress
// reservation.
func (p *inputPty) assertInProgressReservation() {
	conn, err := connect(p.databaseURL)
	if err != nil {
		panic(err)
	}
	defer conn.close() //nolint:errcheck // a read-only check connection
	for _, attempt := range p.hooks.reservedAttempts() {
		receipt, isReceipt := attempt.(attemptKeyReceipt)
		if !isReceipt {
			return
		}
		input, err := getInput(context.Background(), conn, receipt.id)
		if err != nil {
			panic(err)
		}
		if input.state == inputStateInProgress {
			return
		}
	}
	panic("a programmatic stage writes only for an in-progress reservation")
}

func (p *inputPty) tryWrite(data []byte) (int, error) {
	if len(data) == 0 {
		panic("a PTY write offers bytes")
	}
	p.offers = append(p.offers, slices.Clone(data))
	if failure, set := p.failNext.get(); set {
		p.failNext = none[error]()
		return 0, failure
	}
	if failure, set := p.failure.get(); set {
		return 0, failure
	}
	belongsToProgram := bytes.HasPrefix(p.expectedProgram[p.programOffset:], data)
	if !belongsToProgram && !bytes.Equal(data, []byte(pasteClose)) {
		p.manual = append(p.manual, data...)
		p.written = append(p.written, data...)
		p.echoManual(data)
		return len(data), nil
	}
	p.assertInProgressReservation()
	if p.hooks.count("before") == 0 {
		panic("a programmatic stage follows its BeforeWrite checkpoint")
	}
	startsProgram := belongsToProgram && p.programOffset == 0
	p.stages = append(p.stages, timedStage{at: p.clock.now(), bytes: slices.Clone(data)})
	accepted := len(data)
	if limit, set := p.short.get(); set {
		accepted = min(limit, len(data))
	}
	p.written = append(p.written, data[:accepted]...)
	if belongsToProgram {
		p.programOffset += accepted
	}
	completed := belongsToProgram && p.programOffset == len(p.expectedProgram)
	if completed {
		p.program = append(p.program, slices.Clone(p.expectedProgram))
		p.programOffset = 0
	}
	p.echoProgram(data[:accepted], startsProgram, completed)
	return accepted, nil
}

func (p *inputPty) resize(terminalSize) error {
	return nil
}

func (p *inputPty) close() error {
	return nil
}

type runSelection struct {
	backend       codingAgent
	nativeVersion opt[string]
}

// inputRunnerFixture is a run owned by this process as its runner, with a started writer,
// observation controller and transport on one virtual clock; this process stands in as the
// recorded native child.
type inputRunnerFixture struct {
	*writerFixture
	writer     *eventWriter
	controller *observationController
	transport  *transport
	clock      *testClock
	until      time.Time
}

func newInputRunnerFixture(t *testing.T, catalog *profileCatalog, selection opt[runSelection], size terminalSize, limits transportLimits, until time.Time, clockLead time.Duration) *inputRunnerFixture {
	t.Helper()
	f := newWriterFixture(t)
	if chosen, ok := selection.get(); ok {
		coverage := catalog.capabilities(chosen.backend).detection
		if _, err := f.conn.execute("UPDATE member_runs SET backend=?1,native_version=?2,coverage=?3 WHERE run_id=?4", chosen.backend.name(), chosen.nativeVersion, coverage.asStr(), f.run.runID); err != nil {
			t.Fatal(err)
		}
		if _, err := f.conn.execute("UPDATE member_placements SET coding_agent=?1 WHERE member_id=?2", chosen.backend.name(), f.run.memberID); err != nil {
			t.Fatal(err)
		}
	}
	refreshWriterRun(t, f)
	budget := max(time.Until(until), 0)
	c := testClockStartingAt(time.Now().Add(clockLead))
	until = c.now().Add(budget)
	controller := newController(t, f.run, catalog)
	writer, err := eventWriterStartWorker(f.url, f.run, f.lock, yieldingClock{c}, newRunnerDiagnostics(), nil)
	if err != nil {
		t.Fatal(err)
	}
	writer.setDeadline(until)
	identity := currentProcess(t)
	if _, err := f.conn.execute("UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at=?3 WHERE run_id=?4", identity.pid, identity.startIdentity, wallStamp(c), f.run.runID); err != nil {
		t.Fatal(err)
	}
	refreshWriterRun(t, f)
	if f.run.status != runStatusRunning {
		t.Fatalf("status %v", f.run.status)
	}
	tr, err := newTransport(size, limits, c)
	if err != nil {
		t.Fatal(err)
	}
	snapshot := tr.snapshot()
	if err := controller.start(f.run, &snapshot, writer, c); err != nil {
		t.Fatal(err)
	}
	return &inputRunnerFixture{writerFixture: f, writer: writer, controller: controller, transport: tr, clock: c, until: until}
}

func finishWriter(t *testing.T, writer *eventWriter, until time.Time) {
	t.Helper()
	pending, unsaved := writer.finish(until)
	status := writer.status()
	if pending != 0 || unsaved != 0 || status.diagnostic != none[string]() {
		t.Errorf("writer cleanup (%d, %d): %+v", pending, unsaved, status)
	}
}

// inputHost is one native child's input coordinator, transport and observation on a virtual clock.
type inputHost struct {
	t                    *testing.T
	fixture              *inputRunnerFixture
	coordinator          *inputCoordinator
	human                *fakeInput
	pty                  *inputPty
	output               *fakeDisplay
	hooks                *inputTestHooks
	until                time.Time
	stageSettle          time.Duration
	confirmSubmissions   bool
	confirmationFrame    []byte
	idleFrame            []byte
	confirmationPrograms [][]byte
	confirmedBytes       int
	expires              uint64
	clock                *testClock
	tracksRealTime       bool
}

func (h *inputHost) conn() *dbConn {
	return h.fixture.conn
}

func (h *inputHost) run() runRecord {
	return h.fixture.run
}

// childOutput queues data as one read of native child output.
func (h *inputHost) childOutput(data []byte) {
	h.pty.child.push(data)
}

// typeHuman queues data as one read of human terminal input.
func (h *inputHost) typeHuman(data []byte) {
	h.human.push(data)
}

func newInputHost(t *testing.T) *inputHost {
	t.Helper()
	return newInputHostUntil(t, time.Now().Add(5*time.Second))
}

func newInputHostUntil(t *testing.T, until time.Time) *inputHost {
	t.Helper()
	return buildInputHost(t, inputCatalog(t), encodedLiteral(), metadataProfile(), until)
}

func newCatalogHost(t *testing.T, catalog *profileCatalog, expectedProgram []byte, profile hostProfile) *inputHost {
	t.Helper()
	return buildInputHost(t, catalog, expectedProgram, profile, time.Now().Add(5*time.Second))
}

func buildInputHost(t *testing.T, catalog *profileCatalog, expectedProgram []byte, profile hostProfile, until time.Time) *inputHost {
	t.Helper()
	if !time.Now().Before(until) {
		t.Fatal("original fixture construction deadline")
	}
	var painter composerPainter
	if bytes.Equal(profile.screen, []byte(ordinaryScreen)) {
		painter = syntheticComposerPainter()
	} else if native, ok := nativeComposerPainter(t, profile.backend, profile.size, profile.screen); ok {
		painter = native
	}
	composerSelected := catalog.capabilities(profile.backend).composer == coverageTested
	fixture := newInputRunnerFixture(t, catalog, some(runSelection{backend: profile.backend, nativeVersion: profile.nativeVersion}), profile.size,
		transportLimits{inputBytes: 4096, displayBytes: 4096, bytesPerTick: 4096}, until, profile.clockLead)
	expiration, err := inputExpirationCapture(fixture.until, fixture.clock)
	if err != nil {
		t.Fatal(err)
	}
	hooks := &inputTestHooks{until: fixture.until}
	coordinator, err := newInputCoordinator(context.Background(), fixture.conn, fixture.run, fixture.lock,
		testInputServices(catalog, hooks, profile.stageSettle, "ordinary Prompt", fixture.clock), fixture.until)
	if err != nil {
		t.Fatal(err)
	}
	host := &inputHost{
		t:           t,
		fixture:     fixture,
		coordinator: coordinator,
		human:       &fakeInput{},
		pty: &inputPty{
			t: t, child: newFakeInput(profile.screen, max(len(profile.screen), 1)), expectedProgram: expectedProgram,
			databaseURL: fixture.url, hooks: hooks, clock: fixture.clock, painter: painter,
		},
		output:             &fakeDisplay{},
		hooks:              hooks,
		until:              fixture.until,
		stageSettle:        profile.stageSettle,
		confirmSubmissions: true,
		idleFrame:          slices.Clone(profile.screen),
		confirmationFrame:  slices.Clone(profile.screen),
		expires:            expiration.monotonicNs(),
		clock:              fixture.clock,
		tracksRealTime:     profile.clockLead == 0,
	}
	t.Cleanup(host.finish)
	host.drain()
	if composerSelected {
		if eligibility := fixture.controller.health(fixture.writer).Eligibility; eligibility != (inputEligibilityComposerEmpty{}) {
			t.Fatalf("the host starts on an empty composer: %v", eligibility)
		}
	}
	return host
}

func (h *inputHost) identity() controlIdentity {
	return controlIdentityFromRun(h.run())
}

func (h *inputHost) request(key string) inputControlRequest {
	return inputControlRequest{
		Identity:        h.identity(),
		ExpiresAtMonoNS: h.expires,
		Operation:       submitPromptOperation{input: submitInput{OperationKey: key, InputKind: inputKindPrompt, Literal: ordinaryLiteral}},
	}
}

func (h *inputHost) show(frame []byte) {
	h.childOutput(frame)
	h.drain()
	if bytes.Equal(frame, h.idleFrame) {
		h.pty.clearComposer()
	}
	if h.fixture.controller.health(h.fixture.writer).Eligibility == (inputEligibilityComposerEmpty{}) {
		h.pty.clearComposer()
		h.confirmationFrame = frame
	}
}

// catchUp moves virtual time up to real time on a host whose clock starts at real time, so a
// storage deadline derived from it is never already past on the real-time database budget.
func (h *inputHost) catchUp() {
	if !h.tracksRealTime {
		return
	}
	origin := h.clock.now().Add(-h.clock.elapsed())
	if elapsed := time.Since(origin); elapsed > h.clock.elapsed() {
		h.clock.setElapsed(elapsed)
	}
}

func (h *inputHost) quiesce() (inputDrain, *inputFailure) {
	return progressWithManual(h.coordinator, h.fixture.transport, func(t *transport) (inputDrain, error) {
		return t.quiesceForInput(h.human, h.pty, h.output, h.fixture.controller.observer(h.fixture.writer, h.clock), h.until)
	})
}

func (h *inputHost) drain() {
	h.t.Helper()
	for {
		h.catchUp()
		if !h.clock.now().Before(h.until) {
			h.t.Fatal("the fixture becomes healthy and quiescent within its original deadline")
		}
		awaitWriter(h.t, h.fixture.writer)
		snapshot := h.fixture.transport.snapshot()
		if _, err := h.fixture.controller.poll(&snapshot, h.fixture.writer, h.clock); err != nil {
			h.t.Fatal(err)
		}
		result, failure := h.quiesce()
		if failure != nil {
			h.t.Fatal(failure.asError())
		}
		if result == inputDrainQuiescent {
			return
		}
		h.clock.sleep(time.Millisecond)
	}
}

func (h *inputHost) quiesceOnce() inputDrain {
	h.t.Helper()
	result, failure := h.quiesce()
	if failure != nil {
		h.t.Fatal(failure.asError())
	}
	return result
}

func (h *inputHost) advance() opt[inputControlReply] {
	h.t.Helper()
	reply, failure := h.tryAdvance()
	if failure != nil {
		h.t.Fatal(failure.asError())
	}
	return reply
}

func (h *inputHost) confirmAfterPrograms(programs ...[]byte) {
	if len(h.confirmationPrograms) != 0 || len(programs) == 0 || slices.ContainsFunc(programs, func(p []byte) bool { return len(p) == 0 }) {
		panic("confirmation programs are non-empty and set once")
	}
	h.confirmationPrograms = append(h.confirmationPrograms, programs...)
	h.confirmedBytes = len(h.pty.written)
}

func (h *inputHost) tryAdvance() (opt[inputControlReply], *inputFailure) {
	h.catchUp()
	ordinary := h.coordinator.pending != nil && h.coordinator.pending.composer() != nil
	completed := len(h.pty.program)
	reply, failure := h.coordinator.advance(context.Background(), h.conn(), h.fixture.transport, h.pty, h.fixture.controller, h.until)
	var completeProgram bool
	if len(h.confirmationPrograms) > 0 {
		program := h.confirmationPrograms[0]
		completeProgram = len(h.pty.written) >= h.confirmedBytes+len(program) && bytes.HasSuffix(h.pty.written, program)
	} else {
		completeProgram = len(h.pty.program) > completed
	}
	if ordinary && h.confirmSubmissions && completeProgram {
		if len(h.confirmationPrograms) > 0 {
			h.confirmationPrograms = h.confirmationPrograms[1:]
		}
		h.confirmedBytes = len(h.pty.written)
		h.pty.typed = nil
		h.childOutput(slices.Clone(h.confirmationFrame))
	}
	return reply, failure
}

func (h *inputHost) handle(request inputControlRequest) (opt[inputControlReply], *inputFailure) {
	return h.handleUntil(request, h.until)
}

func operationKeyOf(operation inputControlOperation) opt[string] {
	switch operation := operation.(type) {
	case submitPromptOperation:
		return some(operation.input.OperationKey)
	case submitExecOperation:
		return some(operation.input.OperationKey)
	case permissionInspectOperation, permissionRespondOperation, receiptOperation:
		return none[string]()
	default:
		panic("invalid inputControlOperation")
	}
}

// handleUntil applies the request's own expiration first, as the control service does before it
// hands a received request to the coordinator.
func (h *inputHost) handleUntil(request inputControlRequest, deadline time.Time) (opt[inputControlReply], *inputFailure) {
	err := runtimeCheckDeadline(h.clock.now(), deadline)
	if err == nil {
		var received time.Time
		received, err = receiveInputExpiration(request.ExpiresAtMonoNS, h.clock.now(), h.clock)
		if err == nil && received.Before(deadline) {
			deadline = received
		}
	}
	if err != nil {
		return none[inputControlReply](), h.coordinator.runFailure(cafleetErrorOf(err), none[int64](), none[int64](), operationKeyOf(request.Operation))
	}
	return h.coordinator.handleReceived(context.Background(), h.conn(), request, h.fixture.controller, deadline)
}

func (h *inputHost) receipt(action receiptAction) (inputControlReply, *inputFailure) {
	request := h.request("unused-receipt-key")
	request.Operation = receiptOperation{action: action}
	reply, failure := h.handle(request)
	if failure != nil {
		return nil, failure
	}
	value, ok := reply.get()
	if !ok {
		panic("receipt actions reply immediately")
	}
	return value, nil
}

func (h *inputHost) respond(input permissionRespondInput) (opt[inputControlReply], *inputFailure) {
	return h.respondUntil(input, h.until)
}

func (h *inputHost) respondUntil(input permissionRespondInput, deadline time.Time) (opt[inputControlReply], *inputFailure) {
	request := h.request("unused-response-key")
	request.Operation = permissionRespondOperation{input: input}
	return h.coordinator.handleReceived(context.Background(), h.conn(), request, h.fixture.controller, deadline)
}

func (h *inputHost) mustHandle(request inputControlRequest) opt[inputControlReply] {
	h.t.Helper()
	reply, failure := h.handle(request)
	if failure != nil {
		h.t.Fatal(failure.asError())
	}
	return reply
}

// arm submits key and drives turns until the real reservation passes BeforeWrite, returning its
// input id.
func (h *inputHost) arm(key string) int64 {
	h.t.Helper()
	if reply := h.mustHandle(h.request(key)); reply != none[inputControlReply]() {
		h.t.Fatalf("the submission waits: %v", reply)
	}
	for range 8 {
		h.drain()
		if h.advance() != none[inputControlReply]() || len(h.pty.program) != 0 {
			h.t.Fatal("arming writes no program")
		}
		if h.hooks.count("before") == 1 {
			id := h.hooks.inputID(h.t)
			if input := mustGetInput(h.t, h.conn(), id); input.state != inputStateInProgress {
				h.t.Fatalf("input %d is %v", id, input.state)
			}
			return id
		}
		if h.hooks.count("reserved") == 1 {
			if h.advance() != none[inputControlReply]() || h.hooks.count("before") != 0 {
				h.t.Fatal("reservation consumes its old drain proof")
			}
		}
	}
	h.t.Fatal("real reservation and BeforeWrite reach Armed within finite turns")
	return 0
}

func (h *inputHost) finish() {
	finishWriter(h.t, h.fixture.writer, h.until)
}

func (h *inputHost) placement() placement {
	h.t.Helper()
	return requireMember(h.t, h.conn(), h.run().memberID, h.run().fleetID).placement
}

func mustGetInput(t *testing.T, conn *dbConn, inputID int64) inputOperation {
	t.Helper()
	input, err := getInput(context.Background(), conn, inputID)
	if err != nil {
		t.Fatal(err)
	}
	return input
}

// settle submits request, then drives drained turns until its first reply.
func settle(t *testing.T, h *inputHost, request inputControlRequest) submissionResult {
	t.Helper()
	if reply := h.mustHandle(request); reply != none[inputControlReply]() {
		return submitted(t, reply)
	}
	for {
		if !h.clock.now().Before(h.until) {
			t.Fatal("the submission replies in time")
		}
		h.drain()
		if reply := h.advance(); reply != none[inputControlReply]() {
			return submitted(t, reply)
		}
		h.clock.advance(5 * time.Millisecond)
	}
}

func submitted(t *testing.T, reply opt[inputControlReply]) submissionResult {
	t.Helper()
	value, _ := reply.get()
	result, ok := value.(submissionResult)
	if !ok {
		t.Fatalf("reply %#v is a submission result", value)
	}
	return result
}

func storedDeliverySelection(t *testing.T, h *inputHost, inputID int64) (opt[string], opt[string]) {
	t.Helper()
	rows := tableRows(t, h.conn(), "SELECT delivery_method,delivery_reason FROM member_input_operations WHERE input_id=?1", inputID)
	optional := func(value any) opt[string] {
		if text, ok := value.(string); ok {
			return some(text)
		}
		return none[string]()
	}
	return optional(rows[0][0]), optional(rows[0][1])
}

// manualHarness is the manual-progress fixture: a running run on the bundled catalog with small
// transport limits and a quiet observation controller.
type manualHarness struct {
	*inputRunnerFixture
	t *testing.T
}

func newManualHarness(t *testing.T) *manualHarness {
	t.Helper()
	fixture := newInputRunnerFixture(t, profileCatalogBundled(), none[runSelection](), terminalSize{rows: 24, cols: 80},
		transportLimits{inputBytes: 64, displayBytes: 64, bytesPerTick: 64}, time.Now().Add(4*time.Second), runtimeVirtualLead)
	h := &manualHarness{inputRunnerFixture: fixture, t: t}
	t.Cleanup(func() { finishWriter(t, fixture.writer, fixture.until) })
	return h
}

func (h *manualHarness) coordinator() *inputCoordinator {
	h.t.Helper()
	coordinator, err := newInputCoordinator(context.Background(), h.conn, h.run, h.lock,
		testInputServices(profileCatalogBundled(), nil, stageSettle, "manual progress", h.clock), h.until)
	if err != nil {
		h.t.Fatal(err)
	}
	return coordinator
}

func (h *manualHarness) observer() screenObserver {
	return h.controller.observer(h.writer, h.clock)
}

func (h *manualHarness) quietControl() {
	h.t.Helper()
	for {
		snapshot := h.transport.snapshot()
		if _, err := h.controller.poll(&snapshot, h.writer, h.clock); err != nil {
			h.t.Fatal(err)
		}
		if health := h.controller.health(h.writer); !health.Degraded && health.PendingCount == 0 {
			break
		}
		if !h.clock.now().Before(h.until) {
			h.t.Fatalf("writer health: %+v", h.controller.health(h.writer))
		}
		h.clock.advance(time.Millisecond)
	}
	result, err := h.transport.quiesceForInput(&fakeInput{}, newFakePty(nil, 1), &fakeDisplay{}, h.observer(), h.until)
	if err != nil || result != inputDrainQuiescent {
		h.t.Fatalf("quiet control: %v %v", result, err)
	}
}

func TestRuntimeInput_ManualProgress_ManualProgressCountsOnlyNewReadsAcrossPendingZeroAndSuccessiveSteps(t *testing.T) {
	t.Parallel()
	h := newManualHarness(t)
	h.quietControl()
	coordinator := h.coordinator()
	if coordinator.manualGeneration != 0 {
		t.Fatalf("manual generation %d", coordinator.manualGeneration)
	}
	human := &fakeInput{}
	human.push([]byte("first"))
	pty := newFakePty(nil, 1)
	pty.blocked = true
	calls := 0
	result, failure := progressWithManual(coordinator, h.transport, func(tr *transport) (inputDrain, error) {
		calls++
		return tr.quiesceForInput(human, pty, &fakeDisplay{}, h.observer(), h.until)
	})
	if failure != nil || result != inputDrainPending || calls != 1 {
		t.Fatalf("drain %v %v calls %d", result, failure, calls)
	}
	assertBytes(t, human.consumed, []byte("first"), "consumed")
	if h.transport.humanGeneration != 5 || coordinator.manualGeneration != 5 || h.transport.pendingInputBytes() != 5 || len(pty.written) != 0 {
		t.Fatalf("generations %d/%d pending %d", h.transport.humanGeneration, coordinator.manualGeneration, h.transport.pendingInputBytes())
	}
	tick := func() transportProgress {
		t.Helper()
		progress, failure := progressWithManual(coordinator, h.transport, func(tr *transport) (transportProgress, error) {
			calls++
			return tr.tick(human, pty, &fakeDisplay{}, h.observer())
		})
		if failure != nil {
			t.Fatal(failure.asError())
		}
		return progress
	}
	if zero := tick(); zero.humanBytes != 0 || calls != 2 || h.transport.humanGeneration != 5 || coordinator.manualGeneration != 5 || h.transport.pendingInputBytes() != 5 {
		t.Fatalf("zero tick %+v", zero)
	}
	human.push([]byte("+next"))
	pty.blocked = false
	next := tick()
	if next.humanBytes != 5 || calls != 3 || h.transport.humanGeneration != 10 || coordinator.manualGeneration != 10 || h.transport.pendingInputBytes() != 0 {
		t.Fatalf("next tick %+v", next)
	}
	assertBytes(t, human.consumed, []byte("first+next"), "consumed")
	assertBytes(t, pty.written, []byte("first+next"), "written")
}

func TestRuntimeInput_ManualProgress_ManualProgressAccountsRealHumanReadBeforeReturningThePtyFailure(t *testing.T) {
	t.Parallel()
	h := newManualHarness(t)
	h.quietControl()
	coordinator := h.coordinator()
	human := &fakeInput{}
	human.push([]byte("typed"))
	pty := newFakePty(nil, 1)
	pty.failure = some(syscall.EPIPE)
	calls := 0
	_, failure := progressWithManual(coordinator, h.transport, func(tr *transport) (transportProgress, error) {
		calls++
		return tr.tick(human, pty, &fakeDisplay{}, h.observer())
	})
	if failure == nil {
		t.Fatal("the actual PTY failure remains the progress result")
	}
	assertBytes(t, human.consumed, []byte("typed"), "consumed")
	if calls != 1 || h.transport.humanGeneration != 5 || coordinator.manualGeneration != 5 || h.transport.pendingInputBytes() != 5 || len(pty.written) != 0 || pty.reads != 0 {
		t.Fatalf("calls %d generations %d/%d", calls, h.transport.humanGeneration, coordinator.manualGeneration)
	}
	if !slices.EqualFunc(pty.offers, offers("typed"), bytes.Equal) {
		t.Fatalf("offers %q", pty.offers)
	}
	if failure.source.kind != errorApp || failure.source.message() != "member runtime: fixture PTY write failure" || failure.source.exitCode() != 1 {
		t.Fatalf("source %v", failure.source)
	}
	if failure.context.runID != some(h.run.runID) || failure.context.interactionID != none[int64]() || failure.context.inputID != none[int64]() || failure.context.operationKey != none[string]() {
		t.Fatalf("context %+v", failure.context)
	}
}

func TestRuntimeInput_ManualProgress_ManualProgressComposesOnlyTheConstructedAccountingErrorBranches(t *testing.T) {
	t.Parallel()
	_, err := combineManualProgress(17, nil, databaseBusyError("accounting only"))
	only := cafleetErrorOf(err)
	if err == nil || only.kind != errorDatabaseBusy || only.message() != "accounting only" || only.exitCode() != 1 {
		t.Fatalf("an accounting error remains visible after successful progress: %v", err)
	}
	_, err = combineManualProgress(struct{}{}, usageError("primary progress"), databaseBusyError("secondary accounting"))
	both := cafleetErrorOf(err)
	if err == nil || both.exitCode() != 2 || both.kind != errorWithDiagnostics || both.cause == nil || both.cause.kind != errorUsage ||
		both.cause.message() != "primary progress" || !slices.Equal(both.diagnostics, []string{"secondary accounting"}) {
		t.Fatalf("dual errors retain the primary cause and accounting diagnostic: %+v", both)
	}
}

type observedCheckpoint struct {
	attempt      attemptKey
	stage        submissionStage
	writtenBytes uint64
	outputSeq    uint64
	deadline     time.Time
}

// expiringReservedHooks lets the Reserved checkpoint run past its deadline, then fails or passes.
type expiringReservedHooks struct {
	t        *testing.T
	fail     bool
	mu       sync.Mutex
	observed opt[observedCheckpoint]
	clock    *testClock
}

func (h *expiringReservedHooks) checkpoint(boundary inputBoundary, state inputCheckpoint, deadline time.Time) error {
	if boundary != inputBoundaryReserved || !h.clock.now().Before(deadline) {
		panic("one live Reserved checkpoint")
	}
	h.mu.Lock()
	if h.observed != none[observedCheckpoint]() {
		panic("one actual reservation checkpoint")
	}
	h.observed = some(observedCheckpoint{attempt: state.attempt, stage: state.stage, writtenBytes: state.writtenBytes, outputSeq: state.outputSeq, deadline: deadline})
	h.mu.Unlock()
	h.clock.advance(max(deadline.Sub(h.clock.now()), 0))
	if h.fail {
		return appError("fixture checkpoint failed after expiry")
	}
	return nil
}

type pendingSnapshot struct {
	operationKey, payloadDigest string
	bytes                       string
	deadline                    time.Time
	phase                       inputPhase
	receiptID                   opt[int64]
}

func (h *inputHost) pendingSnapshot() pendingSnapshot {
	h.t.Helper()
	pending := h.coordinator.pending
	if pending == nil {
		h.t.Fatal("an actual pending writer")
	}
	return pendingSnapshot{operationKey: pending.request.operationKey, payloadDigest: pending.request.payloadDigest, bytes: string(pending.bytes), deadline: pending.deadline, phase: pending.phase, receiptID: pending.receiptID()}
}

func (h *inputHost) retainedCheckpoint(attempt attemptKey) inputCheckpoint {
	h.t.Helper()
	checkpoint, err := h.coordinator.retainedCheckpoint(attempt)
	if err != nil {
		h.t.Fatal(err)
	}
	return checkpoint
}

func (h *inputHost) totalChanges() int64 {
	return h.conn().raw.TotalChanges()
}

func (h *inputHost) assertNoWrite() {
	h.t.Helper()
	if len(h.pty.offers) != 0 || len(h.pty.stages) != 0 || len(h.pty.program) != 0 || len(h.pty.manual) != 0 {
		h.t.Fatalf("offers %q stages %d program %d manual %q", h.pty.offers, len(h.pty.stages), len(h.pty.program), h.pty.manual)
	}
}

func TestRuntimeInput_OrdinaryPrompt_CheckpointHookErrorPrecedesExpiryAndBlocksBeforeWrite(t *testing.T) {
	t.Parallel()
	for _, fail := range []bool{true, false} {
		h := newInputHost(t)
		const key = "prompt:checkpoint-error-expiry"
		cacheCount := len(h.coordinator.operations)
		if reply := h.mustHandle(h.request(key)); reply != none[inputControlReply]() {
			t.Fatalf("reply %v", reply)
		}
		pending := h.pendingSnapshot()
		hooks := &expiringReservedHooks{t: t, fail: fail, clock: h.clock}
		h.coordinator.services.hooks = hooks
		deadline := h.clock.now().Add(150 * time.Millisecond)
		if !deadline.Before(h.until) {
			t.Fatal("the checkpoint deadline lies before the fixture deadline")
		}
		var failure *inputFailure
		for range 8 {
			h.drain()
			reply, attempt := h.coordinator.advance(context.Background(), h.conn(), h.fixture.transport, h.pty, h.fixture.controller, deadline)
			if attempt != nil {
				failure = attempt
				break
			}
			if reply != none[inputControlReply]() {
				t.Fatal("the hook blocks before a reply")
			}
		}
		observed, ok := hooks.observed.get()
		if failure == nil || !ok {
			t.Fatal("the real reservation reaches the configured checkpoint")
		}
		id := attemptKeyReceiptID(observed.attempt)
		if !observed.deadline.Equal(deadline) || h.clock.now().Before(deadline) {
			t.Fatalf("deadline %v, now %v", observed.deadline, h.clock.now())
		}
		if fail {
			if failure.source.kind != errorApp || failure.source.message() != "fixture checkpoint failed after expiry" {
				t.Fatalf("source %v", failure.source)
			}
		} else if failure.source.kind != errorDeadlineExceeded {
			t.Fatalf("source %v", failure.source)
		}
		if failure.context.runID != some(h.run().runID) || failure.context.inputID != some(id) || failure.context.operationKey != some(key) {
			t.Fatalf("context %+v", failure.context)
		}
		if observed.stage != submissionStageZeroBytes || observed.writtenBytes != 0 {
			t.Fatalf("observed %+v", observed)
		}
		retained := h.retainedCheckpoint(observed.attempt)
		if retained.attempt != observed.attempt || retained.stage != observed.stage || retained.writtenBytes != observed.writtenBytes || retained.outputSeq != observed.outputSeq {
			t.Fatalf("retained %+v", retained)
		}
		blocked := h.pendingSnapshot()
		if blocked.operationKey != pending.operationKey || blocked.payloadDigest != pending.payloadDigest || blocked.bytes != pending.bytes || !blocked.deadline.Equal(pending.deadline) ||
			blocked.phase != inputPhaseBlocked || blocked.receiptID != some(id) || len(h.coordinator.operations) != cacheCount+1 {
			t.Fatalf("fail=%v: blocked %+v", fail, blocked)
		}
		durable := mustGetInput(t, h.conn(), id)
		if durable.state != inputStateInProgress {
			t.Fatalf("durable %v", durable.state)
		}
		changes := h.totalChanges()
		if reply, failure := h.tryAdvance(); failure != nil || reply != none[inputControlReply]() {
			t.Fatalf("a blocked attempt stays blocked: %v %v", reply, failure)
		}
		if h.pendingSnapshot() != blocked || mustGetInput(t, h.conn(), id) != durable || h.totalChanges() != changes {
			t.Fatal("a blocked attempt changes nothing")
		}
		h.assertNoWrite()
	}
}

// awaitSubmission drives drained turns with 5 ms virtual steps until the staged write replies.
func (h *inputHost) awaitSubmission() submissionResult {
	h.t.Helper()
	for {
		if !h.clock.now().Before(h.until) {
			h.t.Fatal("the staged write completes in time")
		}
		if reply := h.advance(); reply != none[inputControlReply]() {
			return submitted(h.t, reply)
		}
		h.clock.advance(5 * time.Millisecond)
		h.drain()
	}
}

func TestRuntimeInput_OrdinaryPrompt_PromptReservationAndEachHookRequireAFreshDrainBeforeOneRealWrite(t *testing.T) {
	t.Parallel()
	profile := metadataProfile()
	profile.stageSettle = stageSettle
	h := newCatalogHost(t, inputCatalog(t), encodedLiteral(), profile)
	id := h.arm("prompt:fixture-complete")
	if h.advance() != none[inputControlReply]() || len(h.pty.offers) != 0 {
		t.Fatal("no stage is written without a fresh drain")
	}
	h.drain()
	firstSeq := h.fixture.transport.snapshot().screenSeq
	reply := h.awaitSubmission()
	if reply.Status != submissionStatusAccepted || reply.Stage != submissionStageSubmitted || reply.InputID != some(id) {
		t.Fatalf("reply %+v", reply)
	}
	if !slices.EqualFunc(h.pty.stageBytes(), offers(ordinaryLiteral, "~"), bytes.Equal) {
		t.Fatalf("stages %q", h.pty.stageBytes())
	}
	if gaps := h.pty.stageGaps(); len(gaps) != 1 || gaps[0] < stageSettle {
		t.Fatalf("the submit key follows the production settle: %v", gaps)
	}
	if slices.ContainsFunc(h.pty.offers, func(offer []byte) bool { return bytes.Contains(offer, []byte{0x1b}) }) {
		t.Fatal("a plain prompt writes no escape")
	}
	if method, reason := storedDeliverySelection(t, h, id); method != some("profile") || reason != none[string]() {
		t.Fatalf("selection %v %v", method, reason)
	}
	if !slices.EqualFunc(h.pty.program, [][]byte{encodedLiteral()}, bytes.Equal) || mustGetInput(t, h.conn(), id).state != inputStateAccepted {
		t.Fatalf("program %q", h.pty.program)
	}
	retained := h.retainedCheckpoint(attemptKeyReceipt{id: id})
	if retained.stage != submissionStageSubmitted || retained.writtenBytes != uint64(len(encodedLiteral())) || retained.outputSeq < firstSeq || reply.OutputSeq != some(retained.outputSeq) {
		t.Fatalf("retained %+v reply %+v", retained, reply)
	}
	replay := submitted(t, h.mustHandle(h.request("prompt:fixture-complete")))
	if replay.Status != submissionStatusAccepted || replay.InputID != some(id) || len(h.pty.program) != 1 {
		t.Fatalf("replay %+v", replay)
	}
	if h.hooks.count("reserved") != 1 || h.hooks.count("before") < 1 || h.hooks.count("after") < 1 {
		t.Fatal("each hook ran")
	}
}

func TestRuntimeInput_OrdinaryPrompt_PromptPositiveShortWritesContinueToOneConfirmedSubmission(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	id := h.arm("prompt:fixture-short")
	h.pty.short = some(3)
	reply := finishSubmission(t, h, true)
	if reply.Status != submissionStatusAccepted || reply.InputID != some(id) || mustGetInput(t, h.conn(), id).state != inputStateAccepted {
		t.Fatalf("reply %+v", reply)
	}
	assertBytes(t, h.pty.written, encodedLiteral(), "written")
	checkpoint := h.retainedCheckpoint(attemptKeyReceipt{id: id})
	if checkpoint.writtenBytes != uint64(len(encodedLiteral())) || h.hooks.count("after") <= 1 {
		t.Fatalf("checkpoint %+v", checkpoint)
	}
	h.hooks.mu.Lock()
	defer h.hooks.mu.Unlock()
	var last opt[hookPoint]
	for _, point := range h.hooks.points {
		if point.boundary == "after" && point.attempt == attemptKey(attemptKeyReceipt{id: id}) {
			last = some(point)
		}
	}
	after, ok := last.get()
	if !ok || after.count != checkpoint.writtenBytes || after.seq > checkpoint.outputSeq {
		t.Fatalf("the completed short writes retain their final checkpoint: %+v", after)
	}
}

func TestRuntimeInput_OrdinaryPrompt_PromptFailedWriteAfterAPossibleByteRecordsWriteFailed(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	id := h.arm("prompt:fixture-failed")
	h.pty.failure = some[error](fixtureIOError{kind: syscall.EPIPE, message: "injected PTY write failure"})
	h.drain()
	_, failure := h.tryAdvance()
	if failure == nil || !strings.Contains(failure.source.message(), "injected PTY write failure") {
		t.Fatalf("an actual PTY write failure surfaces: %v", failure)
	}
	stored := mustGetInput(t, h.conn(), id)
	if stored.state != inputStateUncertain || stored.stopCause != some(stopCauseWriteFailed) {
		t.Fatalf("stored %+v", stored)
	}
	h.pty.failure = none[error]()
	replay := submitted(t, h.mustHandle(h.request("prompt:fixture-failed")))
	if replay.Status != submissionStatusUncertain || replay.Reason != some("write_failed") {
		t.Fatalf("replay %+v", replay)
	}
}

func TestRuntimeInput_OrdinaryPrompt_PromptReservationRaceReturnsExistingUncertaintyWithoutWriteAuthority(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	h := newInputHost(t)
	const key = "prompt:interleaved-reservation"
	if reply := h.mustHandle(h.request(key)); reply != none[inputControlReply]() {
		t.Fatalf("reply %v", reply)
	}
	if found, err := findInput(ctx, h.conn(), h.run().runID, key); err != nil || found != none[inputOperation]() {
		t.Fatalf("no receipt yet: %v %v", found, err)
	}
	independent := connectedTo(t, h.fixture.databaseFile(t))
	original, err := reserveInput(ctx, independent, h.run().owner(), newInput{
		operationKey:  key,
		kind:          inputKindPrompt,
		payloadDigest: inputPayloadDigest(h.run().runID, submitInput{OperationKey: key, InputKind: inputKindPrompt, Literal: ordinaryLiteral}),
	})
	if err != nil || original.state != inputStateInProgress {
		t.Fatalf("reserve %+v %v", original, err)
	}
	if found, err := findInput(ctx, h.conn(), h.run().runID, key); err != nil || found != some(original) {
		t.Fatalf("found %v %v", found, err)
	}
	var reply opt[inputControlReply]
	for range 3 {
		h.drain()
		if reply = h.advance(); reply != none[inputControlReply]() {
			break
		}
	}
	if reply == none[inputControlReply]() {
		t.Fatal("the retained reservation resolves within three real drained turns")
	}
	result := submitted(t, reply)
	after := mustGetInput(t, independent, original.inputID)
	if result.InputID != some(original.inputID) || result.Status != submissionStatusUncertain || result.Stage != submissionStageUnknown || len(h.pty.offers) != 0 || after != original {
		t.Fatalf("the existing reservation keeps its history without fresh write authority: %+v offers %q", result, h.pty.offers)
	}
}

func assertGuardRefused(t *testing.T, _ programmaticWrite, err error) {
	t.Helper()
	if err == nil {
		t.Fatal("a genuine guarded write is refused before native I/O")
	}
}

func (h *inputHost) acquireGuard() *inputGuard {
	h.t.Helper()
	guard, acquired, err := h.coordinator.acquireWriteGuard(context.Background(), h.conn(), h.fixture.transport, h.fixture.controller, h.until)
	if err != nil || !acquired {
		h.t.Fatalf("an actual Armed acquisition: %v %v", acquired, err)
	}
	return guard
}

func guardBinding(t *testing.T, h *inputHost, other *transport, wrongBytes, stale bool) {
	t.Helper()
	id := h.arm("prompt:guard-binding")
	h.drain()
	before := mustGetInput(t, h.conn(), id)
	guard := h.acquireGuard()
	if stale {
		if _, err := h.fixture.transport.tick(h.human, h.pty, h.output, h.fixture.controller.observer(h.fixture.writer, h.clock)); err != nil {
			t.Fatal(err)
		}
	}
	data := []byte(ordinaryLiteral)
	if wrongBytes {
		data = []byte("unselected bytes")
	}
	target := h.fixture.transport
	if other != nil {
		target = other
	}
	write, err := target.writeProgrammatic(h.pty, data, guard, h.until)
	assertGuardRefused(t, write, err)
	write, err = h.fixture.transport.writeProgrammatic(h.pty, []byte(ordinaryLiteral), guard, h.until)
	assertGuardRefused(t, write, err)
	if len(h.pty.offers) != 0 {
		t.Fatalf("offers %q", h.pty.offers)
	}
	guard.close()
	point := h.retainedCheckpoint(attemptKeyReceipt{id: id})
	if point.stage != submissionStageZeroBytes || point.writtenBytes != 0 || mustGetInput(t, h.conn(), id) != before {
		t.Fatalf("point %+v", point)
	}
}

func TestRuntimeInput_OrdinaryPrompt_Guards_PromptGenuineGuardBindsTransportTurnAndSelectedBytesAndConsumesRejection(t *testing.T) {
	t.Parallel()
	guardBinding(t, newInputHost(t), newInputHost(t).fixture.transport, false, false)
	guardBinding(t, newInputHost(t), nil, true, false)
	guardBinding(t, newInputHost(t), nil, false, true)
}

func TestRuntimeInput_OrdinaryPrompt_Guards_PromptGuardDropDistinguishesConsumedPreIoFromUnrecordedPossibleIo(t *testing.T) {
	t.Parallel()
	for _, possible := range []bool{false, true} {
		h := newInputHost(t)
		id := h.arm("prompt:guard-drop")
		h.drain()
		before := mustGetInput(t, h.conn(), id)
		h.hooks.mu.Lock()
		hooks := len(h.hooks.points)
		h.hooks.mu.Unlock()
		guard := h.acquireGuard()
		bound := h.until
		if !possible {
			bound = h.clock.now().Add(30 * time.Millisecond)
		}
		effective, err := guard.beginWrite(h.fixture.transport, []byte(ordinaryLiteral), bound)
		if err != nil || effective.After(bound) {
			t.Fatalf("beginWrite %v %v", effective, err)
		}
		if possible {
			guard.markIoPossible()
		} else {
			if _, err := guard.beginWrite(h.fixture.transport, []byte(ordinaryLiteral), h.until); err == nil {
				t.Fatal("successful pre-I/O validation consumes the permit too")
			}
			h.clock.advance(max(effective.Sub(h.clock.now()), 0) + time.Millisecond)
		}
		guard.close()
		point := h.retainedCheckpoint(attemptKeyReceipt{id: id})
		wantStage := submissionStageZeroBytes
		if possible {
			wantStage = submissionStageUnknown
		}
		if point.stage != wantStage || point.writtenBytes != 0 || point.outputSeq != h.fixture.transport.snapshot().screenSeq || len(h.pty.offers) != 0 {
			t.Fatalf("possible=%v: point %+v", possible, point)
		}
		h.hooks.mu.Lock()
		after := len(h.hooks.points)
		h.hooks.mu.Unlock()
		if after != hooks || mustGetInput(t, h.conn(), id) != before {
			t.Fatal("a drop creates no checkpoint hook and leaves durable InProgress unchanged")
		}
	}
}

func TestRuntimeInput_OrdinaryPrompt_PreviewExec_OperationsExecSelectsItsNativeShellPlanAndRequiresItsOwnCoverage(t *testing.T) {
	t.Parallel()
	definition := inputCatalogDefinition()
	jset(definition, fixtureAt(syntheticVersion), "codex", "screens", "synthetic-exec-plan")
	jset(definition, parseTestJSON(`{
		"prompt":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"","submit_hex":"0d","fixture_ids":["synthetic-exec-plan"]},
		"exec":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"2120","submit_hex":"6b6c","fixture_ids":["synthetic-exec-plan"]}
	}`), "codex", "profile", "capabilities", "composer", "submission")
	catalog := mustLoadCatalog(t, definition).withoutSubmission(codingAgentCodex, composerOperationPrompt)
	literal := append([]byte("! "), ordinaryLiteral...)
	h := newCatalogHost(t, catalog, append(slices.Clone(literal), "kl"...), metadataProfile())
	key := fmt.Sprintf("exec:%d:%s", h.run().runID, strings.Repeat("b", 64))
	request := h.request(key)
	request.Operation = submitExecOperation{input: submitInput{OperationKey: key, InputKind: inputKindExec, Literal: ordinaryLiteral}}
	reply := settle(t, h, request)
	if reply.Status != submissionStatusAccepted || reply.Stage != submissionStageSubmitted {
		t.Fatalf("reply %+v", reply)
	}
	inputID, _ := reply.InputID.get()
	row := mustGetInput(t, h.conn(), inputID)
	if row.kind != inputKindExec || row.operationKey != key || row.state != inputStateAccepted {
		t.Fatalf("row %+v", row)
	}
	if !slices.EqualFunc(h.pty.offers, [][]byte{literal, []byte("kl")}, bytes.Equal) {
		t.Fatalf("offers %q", h.pty.offers)
	}
	if method, reason := storedDeliverySelection(t, h, inputID); method != some("profile") || reason != none[string]() {
		t.Fatalf("selection %v %v", method, reason)
	}
}

// turn ends a pending stage settle, runs queue housekeeping, drains and advances once.
func turn(t *testing.T, h *inputHost) opt[submissionResult] {
	t.Helper()
	if pending := h.coordinator.pending; pending != nil {
		if composer := pending.composer(); composer != nil {
			if until, set := composer.settleUntil.get(); set && until.After(h.clock.now()) {
				composer.settleUntil = some(h.clock.now())
			}
		}
	}
	housekeep(t, h)
	h.drain()
	if reply := h.advance(); reply != none[inputControlReply]() {
		return some(submitted(t, reply))
	}
	return none[submissionResult]()
}

func housekeep(t *testing.T, h *inputHost) {
	t.Helper()
	if err := h.coordinator.queueHousekeeping(context.Background(), h.conn(), h.fixture.controller); err != nil {
		t.Fatal(err)
	}
}

// waitStep is the virtual time a waiting helper lets pass between turns.
const waitStep = 20 * time.Millisecond

// finishSubmission drives turns until the submission replies, confirming the submit key with
// the idle screen or ending the post-submit verification.
func finishSubmission(t *testing.T, h *inputHost, confirm bool) submissionResult {
	t.Helper()
	for range 256 {
		if pending := h.coordinator.pending; pending != nil && len(h.pty.offers) > 0 {
			last := h.pty.offers[len(h.pty.offers)-1]
			if bytes.Equal(last, []byte("~")) || bytes.Equal(last, []byte("\r")) {
				if confirm {
					h.childOutput([]byte(ordinaryScreen))
				} else {
					h.setPostSubmitVerificationUntil(h.clock.now())
				}
			}
		}
		if reply, ok := turn(t, h).get(); ok {
			return reply
		}
		h.clock.advance(waitStep)
	}
	t.Fatal("the actual coordinator finishes within bounded turns")
	return submissionResult{}
}

// setPostSubmitVerificationUntil is Rust's test-only set_post_submit_verification_until.
func (h *inputHost) setPostSubmitVerificationUntil(until time.Time) {
	if pending := h.coordinator.pending; pending != nil {
		if composer := pending.composer(); composer != nil && composer.verification != nil {
			composer.verification.until = until
		}
	}
}

var (
	pasteOpenBytes  = []byte(pasteOpen)
	pasteCloseBytes = []byte(pasteClose)
)

func replyInput(t *testing.T, h *inputHost, reply submissionResult) inputOperation {
	t.Helper()
	id, ok := reply.InputID.get()
	if !ok {
		t.Fatal("a written input has a receipt")
	}
	return mustGetInput(t, h.conn(), id)
}

func assertUncertainReply(t *testing.T, h *inputHost, reply submissionResult, cause stopCause) {
	t.Helper()
	input := replyInput(t, h, reply)
	if reply.Status != submissionStatusUncertain || input.state != inputStateUncertain || input.stopCause != some(cause) {
		t.Fatalf("reply %+v input %+v, want uncertain %v", reply, input, cause)
	}
}

func inspectionForm(h *inputHost, operation, focus string) {
	h.show(inspectionFixtureForm(operation, focus))
}

type requestIDs struct {
	eventID, interactionID, messageID, episodeSeq int64
}

func currentIDs(t *testing.T, h *inputHost) opt[requestIDs] {
	t.Helper()
	rows := tableRows(t, h.conn(), "SELECT e.event_id,e.interaction_id,e.message_id,i.episode_seq FROM member_runtime_events e JOIN member_interactions i ON i.interaction_id=e.interaction_id WHERE e.run_id=?1 AND e.kind='interaction_opened' AND i.closed_at IS NULL ORDER BY e.event_id DESC LIMIT 1", h.run().runID)
	if len(rows) == 0 {
		return none[requestIDs]()
	}
	return some(requestIDs{eventID: rows[0][0].(int64), interactionID: rows[0][1].(int64), messageID: rows[0][2].(int64), episodeSeq: rows[0][3].(int64)})
}

func settledIDs(t *testing.T, h *inputHost) requestIDs {
	t.Helper()
	for {
		h.drain()
		if h.fixture.controller.currentRequest != nil {
			if ids, ok := currentIDs(t, h).get(); ok {
				return ids
			}
		}
		if !h.clock.now().Before(h.until) {
			t.Fatal("a stable actual request within the original fixture deadline")
		}
		h.clock.advance(2 * time.Millisecond)
	}
}

// stableForm opens the synthetic permission form: an appearing interaction is what stops a
// staged write.
func stableForm(t *testing.T, h *inputHost) requestIDs {
	t.Helper()
	inspectionForm(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-only")
	return settledIDs(t, h)
}

func untilWritten(t *testing.T, h *inputHost, count int) {
	t.Helper()
	for range 256 {
		if len(h.pty.written) >= count {
			return
		}
		if turn(t, h) != none[submissionResult]() {
			t.Fatal("the literal reaches its checkpoint before replying")
		}
	}
	t.Fatal("the actual writer reaches its checkpoint within bounded turns")
}

func enablePaste(t *testing.T, h *inputHost) {
	t.Helper()
	h.childOutput([]byte("\x1b[?2004h"))
	h.drain()
	if !h.fixture.transport.bracketedPaste() {
		t.Fatal("mode 2004 is on")
	}
}

func literalRequest(h *inputHost, key, literal string) inputControlRequest {
	request := h.request(key)
	request.Operation = submitPromptOperation{input: submitInput{OperationKey: key, InputKind: inputKindPrompt, Literal: literal}}
	return request
}

func asKind(request inputControlRequest, kind inputKind) inputControlRequest {
	if kind == inputKindExec {
		prompt := request.Operation.(submitPromptOperation)
		prompt.input.InputKind = kind
		request.Operation = submitExecOperation(prompt)
	}
	return request
}

func startSubmission(t *testing.T, h *inputHost, key, literal string) {
	t.Helper()
	h.confirmSubmissions = false
	if reply := h.mustHandle(literalRequest(h, key, literal)); reply != none[inputControlReply]() {
		t.Fatalf("reply %v", reply)
	}
}

func framingProgram(literal string, bracketed bool) []byte {
	var program []byte
	if bracketed {
		program = append(program, pasteOpenBytes...)
	}
	program = append(program, literal...)
	if bracketed {
		program = append(program, pasteCloseBytes...)
	}
	return append(program, '~')
}

func configuredCatalog(t *testing.T, framing string, limit any) *profileCatalog {
	t.Helper()
	value := inputCatalogDefinition()
	for _, operation := range []string{"prompt", "exec"} {
		plan := jget(value, "codex", "profile", "capabilities", "composer", "submission", operation)
		jset(plan, framing, "literal_framing")
		jset(plan, limit, "plain_literal_limit")
	}
	return mustLoadCatalog(t, value)
}

// inspectionComposerDefinition is the inspection fixture with the synthetic composer, its text
// rule and its submission plans.
func inspectionComposerDefinition() any {
	definition := inspectionFixtureDefinition()
	composer := inputCatalogDefinition()
	jset(definition, jget(composer, "codex", "screens", "synthetic-prompt-plan"), "codex", "screens", "synthetic-prompt-plan")
	rules := jget(definition, "codex", "profile", "rules").(*jsonArr)
	rules.items = append(rules.items, jget(composer, "codex", "profile", "rules", 0), syntheticTextRule())
	jset(definition, jget(composer, "codex", "profile", "capabilities", "composer"), "codex", "profile", "capabilities", "composer")
	return definition
}

func permissionConfiguredCatalog(t *testing.T, framing string, limit any) *profileCatalog {
	t.Helper()
	definition := inspectionComposerDefinition()
	for _, operation := range []string{"prompt", "exec"} {
		plan := jget(definition, "codex", "profile", "capabilities", "composer", "submission", operation)
		jset(plan, framing, "literal_framing")
		jset(plan, limit, "plain_literal_limit")
	}
	return mustLoadCatalog(t, definition)
}

func limitOf(maxBytes int) any {
	return parseTestJSON(fmt.Sprintf(`{"unit":"bytes","max":%d}`, maxBytes))
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_PointerInputDuringTheSettlePreservesOrdinarySubmission(t *testing.T) {
	t.Parallel()
	for _, pointer := range []string{"\x1b[<35;10;20M", "\x1b[M !!"} {
		h := newInputHost(t)
		startSubmission(t, h, "prompt:pointer-settle", ordinaryLiteral)
		untilWritten(t, h, len(ordinaryLiteral))
		keyboard := h.fixture.transport.humanGeneration
		h.typeHuman([]byte(pointer))
		reply := finishSubmission(t, h, true)
		if reply.Status != submissionStatusAccepted || h.fixture.transport.humanGeneration != keyboard ||
			h.fixture.transport.pointerGeneration != uint64(len(pointer)) || replyInput(t, h, reply).stopCause != none[stopCause]() {
			t.Fatalf("%q: reply %+v", pointer, reply)
		}
	}
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_PositiveShortLiteralWritesContinueWithoutSettlingBetweenChunks(t *testing.T) {
	t.Parallel()
	literal := strings.Repeat("あ", 4000)
	h := newCatalogHost(t, configuredCatalog(t, "plain", nil), framingProgram(literal, false), metadataProfile())
	h.pty.short = some(1000)
	startSubmission(t, h, "prompt:long-chunks", literal)
	untilWritten(t, h, 1000)
	assertBytes(t, h.pty.written, []byte(literal)[:1000], "first chunk")
	keyboard, pointer := h.fixture.transport.humanGeneration, h.fixture.transport.pointerGeneration
	outputSeq := h.fixture.transport.snapshot().screenSeq
	h.show([]byte("\x1b[2J\x1b[1;1H" + literal[:999]))
	if h.fixture.transport.snapshot().screenSeq <= outputSeq || h.fixture.controller.health(h.fixture.writer).Eligibility == (inputEligibilityComposerEmpty{}) {
		t.Fatal("the echoed prefix is a non-empty composer")
	}
	if h.fixture.transport.humanGeneration != keyboard || h.fixture.transport.pointerGeneration != pointer {
		t.Fatal("no manual input")
	}
	untilWritten(t, h, 2000)
	assertBytes(t, h.pty.written, []byte(literal)[:2000], "second chunk")
	if h.coordinator.pending.composer().settleUntil != none[time.Time]() {
		t.Fatal("no settle between chunks")
	}
	if reply := finishSubmission(t, h, true); reply.Status != submissionStatusAccepted {
		t.Fatalf("reply %+v", reply)
	}
	assertBytes(t, h.pty.written, framingProgram(literal, false), "program")
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_AStopInsideAUtf8ScalarClosesPasteAndRemainsUncertain(t *testing.T) {
	t.Parallel()
	const literal = "あいう"
	h := newCatalogHost(t, permissionConfiguredCatalog(t, "bracketed_paste", nil), framingProgram(literal, true), metadataProfile())
	enablePaste(t, h)
	h.pty.short = some(len(pasteOpen) + 1)
	startSubmission(t, h, "prompt:scalar-prefix", literal)
	untilWritten(t, h, len(pasteOpen)+1)
	h.pty.short = none[int]()
	stableForm(t, h)
	reply := finishSubmission(t, h, false)
	if reply.Status != submissionStatusUncertain || replyInput(t, h, reply).stopCause != some(stopCauseShortWrite) || !bytes.Equal(h.pty.offers[len(h.pty.offers)-1], pasteCloseBytes) {
		t.Fatalf("reply %+v offers %q", reply, h.pty.offers)
	}
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_AStopInsideTheOpeningMarkerDoesNotEmitACloseMarker(t *testing.T) {
	t.Parallel()
	h := newCatalogHost(t, permissionConfiguredCatalog(t, "bracketed_paste", nil), framingProgram(ordinaryLiteral, true), metadataProfile())
	enablePaste(t, h)
	h.pty.short = some(3)
	startSubmission(t, h, "prompt:marker-prefix", ordinaryLiteral)
	untilWritten(t, h, 3)
	h.pty.short = none[int]()
	stableForm(t, h)
	reply := finishSubmission(t, h, false)
	if reply.Status != submissionStatusUncertain || replyInput(t, h, reply).stopCause != some(stopCauseShortWrite) {
		t.Fatalf("reply %+v", reply)
	}
	if slices.ContainsFunc(h.pty.offers, func(offer []byte) bool { return bytes.Equal(offer, pasteCloseBytes) }) {
		t.Fatalf("offers %q", h.pty.offers)
	}
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_AChunkDeadlineAtACompletePrefixRecordsPartialDeadlineExpired(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	h.pty.short = some(3)
	startSubmission(t, h, "prompt:chunk-deadline", ordinaryLiteral)
	untilWritten(t, h, 3)
	h.coordinator.pending.deadline = h.clock.now().Add(100 * time.Millisecond)
	assertUncertainReply(t, h, finishSubmission(t, h, false), stopCauseDeadlineExpired)
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_BracketedFramingIsSelectedOnlyWhileMode2004IsEnabled(t *testing.T) {
	t.Parallel()
	for _, enabled := range []bool{false, true} {
		h := newCatalogHost(t, configuredCatalog(t, "bracketed_paste", nil), framingProgram(ordinaryLiteral, enabled), metadataProfile())
		if enabled {
			enablePaste(t, h)
		}
		startSubmission(t, h, "prompt:mode-selection", ordinaryLiteral)
		if reply := finishSubmission(t, h, true); reply.Status != submissionStatusAccepted {
			t.Fatalf("enabled=%v: reply %+v", enabled, reply)
		}
		assertBytes(t, h.pty.written, framingProgram(ordinaryLiteral, enabled), "program")
	}
}

func assertImmediateLimitRejection(t *testing.T, kind inputKind) {
	t.Helper()
	for _, framing := range []string{"plain", "bracketed_paste"} {
		h := newCatalogHost(t, configuredCatalog(t, framing, limitOf(1)), encodedLiteral(), metadataProfile())
		before := runtimeTableRows(t, h.conn(), "member_input_operations")
		reply := submitted(t, h.mustHandle(asKind(literalRequest(h, "ordinary:over-limit", ordinaryLiteral), kind)))
		size := len(ordinaryLiteral)
		if kind == inputKindExec {
			size += 2
		}
		cause := "the native CLI has not enabled bracketed paste"
		if framing == "plain" {
			cause = "the codex profile has no bracketed-paste delivery"
		}
		want := fmt.Sprintf("the input is %d bytes, above the codex plain-input limit of 1 bytes, and %s", size, cause)
		if reply.Status != submissionStatusRejected || reply.Stage != submissionStageZeroBytes || reply.InputID != none[int64]() ||
			reply.OutputSeq != none[uint64]() || reply.Delivery != none[deliveryProjection]() || reply.Reason != some(want) {
			t.Fatalf("%s: reply %+v", framing, reply)
		}
		if len(h.pty.offers) != 0 || !rowsEqual(runtimeTableRows(t, h.conn(), "member_input_operations"), before) {
			t.Fatalf("%s: a pre-write refusal writes nothing", framing)
		}
	}
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_ImmediateOverLimitPromptIsRefusedBeforeReceiptOrBytes(t *testing.T) {
	t.Parallel()
	assertImmediateLimitRejection(t, inputKindPrompt)
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_ImmediateOverLimitExecIsRefusedBeforeReceiptOrBytes(t *testing.T) {
	t.Parallel()
	assertImmediateLimitRejection(t, inputKindExec)
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_ZeroByteReselectionRefusesWithTheActualReadyReceiptAndProjection(t *testing.T) {
	t.Parallel()
	for _, kind := range []inputKind{inputKindPrompt, inputKindExec} {
		h := newCatalogHost(t, configuredCatalog(t, "bracketed_paste", limitOf(1)), encodedLiteral(), metadataProfile())
		enablePaste(t, h)
		if reply := h.mustHandle(asKind(literalRequest(h, "ordinary:mode-disabled", ordinaryLiteral), kind)); reply != none[inputControlReply]() {
			t.Fatalf("reply %v", reply)
		}
		if turn(t, h) != none[submissionResult]() || len(h.pty.offers) != 0 {
			t.Fatal("the first turn writes nothing")
		}
		id, _ := h.coordinator.pending.receiptID().get()
		before := mustGetInput(t, h.conn(), id)
		retained, ok := h.coordinator.operations[attemptKeyReceipt{id: id}]
		if before.state != inputStateInProgress || !ok || retained.stage != submissionStageZeroBytes || retained.writtenBytes != 0 || retained.leased {
			t.Fatalf("before %+v retained %+v", before, retained)
		}
		h.show(append([]byte("\x1b[?2004l"), ordinaryScreen...))
		if h.fixture.transport.bracketedPaste() {
			t.Fatal("mode 2004 is off")
		}
		reply := finishSubmission(t, h, false)
		after := mustGetInput(t, h.conn(), id)
		size := len(ordinaryLiteral)
		if kind == inputKindExec {
			size += 2
		}
		want := fmt.Sprintf("the input is %d bytes, above the codex plain-input limit of 1 bytes, and the native CLI has not enabled bracketed paste", size)
		if reply.Status != submissionStatusRejected || reply.Stage != submissionStageZeroBytes || reply.InputID != some(id) || after.state != inputStateReady ||
			after.operationKey != before.operationKey || after.payloadDigest != before.payloadDigest || after.kind != kind || after.disposition != none[inputDisposition]() ||
			reply.Delivery != some(after.delivery()) || reply.Reason != some(want) || len(h.pty.offers) != 0 {
			t.Fatalf("kind %v: reply %+v after %+v", kind, reply, after)
		}
	}
}

// afterNativeSubmit writes a native verification literal on a host replaying the backend's empty
// composer at fixtureID's geometry, then runs body with the submit key awaiting confirmation.
func afterNativeSubmit(t *testing.T, fixtureID string, body func(h *inputHost)) {
	t.Helper()
	backend, file := bundledFile(fixtureID)
	screens := jget(file, "screens").(*jsonObj)
	fixture := jget(screens, fixtureID)
	nativeVersion := jget(fixture, "native_version").(string)
	readsEmpty := func(screen any) bool {
		replayed := replayNativeFixture(testJSONText(screen), func(*[]string) {}, fixtureAlternate(screen), none[[2]uint16]())
		return observeBundled(t, backend, nativeVersion, &replayed).kind == observationKindComposerEmpty
	}
	sameGeometry := func(id string) bool {
		candidate := jget(screens, id)
		return !strings.Contains(id, "-director-") && testJSONText(jget(candidate, "native_version")) == testJSONText(jget(fixture, "native_version")) &&
			testJSONText(jget(candidate, "rows")) == testJSONText(jget(fixture, "rows")) && testJSONText(jget(candidate, "cols")) == testJSONText(jget(fixture, "cols"))
	}
	var ordered []string
	for _, id := range screens.keys() {
		if sameGeometry(id) && strings.Contains(id, "-composer-prompt-idle-mode-2004-") {
			ordered = append(ordered, id)
		}
	}
	for _, id := range screens.keys() {
		if sameGeometry(id) {
			ordered = append(ordered, id)
		}
	}
	index := slices.IndexFunc(ordered, func(id string) bool { return readsEmpty(jget(screens, id)) })
	if index < 0 {
		t.Fatalf("%s: its backend and geometry have a captured screen its profile reads as an empty composer", fixtureID)
	}
	operation := composerOperationPrompt
	kind := inputKindPrompt
	if strings.Contains(fixtureID, "-composer-exec-") {
		operation, kind = composerOperationExec, inputKindExec
	} else if !strings.Contains(fixtureID, "-composer-prompt-") {
		t.Fatalf("%s names its operation", fixtureID)
	}
	catalog := profileCatalogBundled()
	plan, ok := catalog.composerPlan(backend, operation)
	const literal = "printf native-verification"
	if !ok || !plan.plainLiteralAllowed(literal) {
		t.Fatalf("%s: plan", fixtureID)
	}
	stages, err := plan.stages(literal, literalFramingPlain)
	if err != nil {
		t.Fatal(err)
	}
	expected := slices.Concat(stages[0], stages[1])
	number := func(key string) uint16 {
		value, err := strconv.ParseUint(string(jget(fixture, key).(jsonNumber)), 10, 16)
		if err != nil {
			t.Fatal(err)
		}
		return uint16(value)
	}
	profile := metadataProfile()
	profile.backend = backend
	profile.nativeVersion = some(nativeVersion)
	profile.size = terminalSize{rows: number("rows"), cols: number("cols")}
	profile.screen = nativeFixtureBytes(bundledScreen(ordered[index]))
	h := newCatalogHost(t, catalog, expected, profile)
	h.confirmSubmissions = false
	if h.fixture.transport.bracketedPaste() {
		t.Fatal("native screen replay leaves mode 2004 off")
	}
	request := h.request("native:verification")
	input := submitInput{OperationKey: "native:verification", InputKind: kind, Literal: literal}
	request.Operation = submitPromptOperation{input: input}
	if kind == inputKindExec {
		request.Operation = submitExecOperation{input: input}
	}
	if reply := h.mustHandle(request); reply != none[inputControlReply]() {
		t.Fatalf("reply %v", reply)
	}
	untilWritten(t, h, len(expected))
	assertBytes(t, h.pty.written, expected, fixtureID)
	if h.coordinator.pending == nil {
		t.Fatalf("%s: the complete submit key awaits native confirmation", fixtureID)
	}
	body(h)
}

func nativeVerificationReply(t *testing.T, h *inputHost) submissionResult {
	t.Helper()
	for range 256 {
		if reply, ok := turn(t, h).get(); ok {
			return reply
		}
	}
	t.Fatal("native verification completes within bounded real coordinator turns")
	return submissionResult{}
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_EveryAuthoritativeRetainedNativeSnapshotLeavesSubmitUnconfirmed(t *testing.T) {
	t.Parallel()
	for _, fixtureID := range []string{
		"claude-2.1.319-composer-exec-plain-41-args-literal-tapped-v2-herdr-119-68",
		"claude-2.1.319-composer-exec-plain-41-args-literal-tapped-v2-tmux-120-68",
		"claude-2.1.319-composer-prompt-plain-41-literal-tapped-v2-herdr-119-33",
		"claude-2.1.319-composer-prompt-plain-41-literal-tapped-v2-tmux-120-33",
		"codex-0.167.0-composer-exec-plain-42-args-literal-tapped-v2-tmux-120-39",
		"codex-0.166.0-composer-exec-plain-42-literal-herdr-092-1",
		"codex-0.166.0-composer-exec-plain-42-literal-tmux-120-1",
		"codex-0.167.0-composer-exec-plain-42-literal-herdr-092-1",
		"codex-0.167.0-composer-exec-plain-42-literal-tmux-120-1",
		"codex-0.167.0-composer-prompt-plain-41-literal-herdr-092-1",
		"codex-0.167.0-composer-prompt-plain-41-literal-tmux-120-1",
		"hermes-2026.9.24-composer-exec-plain-41-args-literal-tapped-v2-herdr-119-99",
		"hermes-2026.9.24-composer-exec-plain-41-args-literal-tapped-v2-tmux-120-100",
		"hermes-2026.9.24-composer-prompt-one-literal-tmux-120-1",
		"opencode-2.2.5-composer-exec-plain-41-args-literal-tapped-v2-herdr-119-68",
		"opencode-2.2.5-composer-exec-plain-41-args-literal-tapped-v2-tmux-120-68",
		"opencode-2.2.5-composer-prompt-plain-41-literal-tapped-v2-herdr-119-33",
		"opencode-2.2.5-composer-prompt-plain-41-literal-tapped-v2-tmux-120-33",
		"pi-1.5.0-composer-exec-plain-41-args-literal-tapped-v2-herdr-119-69",
		"pi-1.5.0-composer-prompt-plain-41-literal-tapped-v2-herdr-119-33",
		"pi-1.5.0-composer-prompt-plain-41-literal-tapped-v2-tmux-120-33",
		"codex-0.167.0-composer-prompt-plain-1025-post-1900ms-tapped-v2-tmux-120-37",
		"codex-0.167.0-composer-exec-plain-1025-args-post-1900ms-tapped-v2-tmux-120-37",
		"hermes-2026.9.24-composer-prompt-plain-4000-max-preview-chunks-post-500ms-tapped-v2-tmux-120-44",
		"hermes-2026.9.24-composer-prompt-plain-4000-max-preview-chunks-post-500ms-tapped-v2-herdr-119-43",
		"hermes-2026.9.24-composer-exec-plain-4000-args-max-preview-chunks-post-500ms-tapped-v2-tmux-120-94",
		"hermes-2026.9.24-composer-exec-plain-4000-args-max-preview-chunks-post-500ms-tapped-v2-herdr-119-93",
		"codex-0.167.0-composer-prompt-retained-authoritative-no-submit-tapped-v2-tmux-120-2",
		"pi-1.5.0-composer-exec-retained-authoritative-no-submit-tapped-v2-tmux-120-2",
	} {
		afterNativeSubmit(t, fixtureID, func(h *inputHost) {
			h.show(nativeFixtureBytes(bundledScreen(fixtureID)))
			if h.fixture.controller.health(h.fixture.writer).Eligibility == (inputEligibilityComposerEmpty{}) {
				t.Fatalf("%s reads as an empty composer", fixtureID)
			}
			h.setPostSubmitVerificationUntil(h.clock.now())
			reply := nativeVerificationReply(t, h)
			assertUncertainReply(t, h, reply, stopCauseSubmitUnconfirmed)
			if reply.Stage != submissionStageSubmitted {
				t.Fatalf("%s: stage %v", fixtureID, reply.Stage)
			}
		})
	}
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_MeasuredWorkingComposersConfirmWrittenSubmitKeys(t *testing.T) {
	t.Parallel()
	for _, fixtureID := range []string{
		"pi-1.5.0-composer-exec-plain-4000-args-max-preview-chunks-post-1900ms-tapped-v2-tmux-120-61",
		"pi-1.5.0-composer-exec-plain-4000-args-max-preview-chunks-post-1900ms-tapped-v2-herdr-119-61",
		"hermes-2026.9.24-composer-prompt-plain-40-post-1900ms-tapped-v2-tmux-120-7",
		"hermes-2026.9.24-composer-prompt-plain-40-post-1900ms-tapped-v2-herdr-119-7",
	} {
		afterNativeSubmit(t, fixtureID, func(h *inputHost) {
			h.show(nativeFixtureBytes(bundledScreen(fixtureID)))
			reply := nativeVerificationReply(t, h)
			if reply.Status != submissionStatusAccepted || reply.Stage != submissionStageSubmitted || replyInput(t, h, reply).stopCause != none[stopCause]() {
				t.Fatalf("%s: reply %+v", fixtureID, reply)
			}
		})
	}
}

// sweepPreview sends body to the run's member and drives the coordinator's receipt-free sweep
// preview of it to its reply, confirming each submit with the idle screen.
func sweepPreview(t *testing.T, h *inputHost, body string, program func(messageRecord) []byte) (messageRecord, submissionResult) {
	t.Helper()
	h.confirmSubmissions = false
	message, err := sendMessage(context.Background(), h.conn(), h.run().directorMemberID, h.run().memberID, body)
	if err != nil {
		t.Fatal(err)
	}
	h.pty.expectProgram(program(message))
	reply, failure := h.coordinator.handlePreview(context.Background(), h.conn(), message, h.fixture.controller, h.until)
	if failure != nil {
		t.Fatal(failure.asError())
	}
	if reply != none[inputControlReply]() {
		return message, submitted(t, reply)
	}
	return message, finishSubmission(t, h, true)
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_ASweepPreviewRendersThePointerFormOverThePlainLimitAndTheFallbackClauseWithoutAPlan(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name           string
		catalog        *profileCatalog
		body           string
		line           func(messageRecord) string
		prefix, submit string
	}{
		{"a preview over the plain limit types its pointer form", configuredCatalog(t, "plain", limitOf(1024)), strings.Repeat("あ", 3000), pointerPreviewLine, "", "~"},
		{"a run without a submission plan types the fallback line with its dismissal clause",
			inputCatalog(t).withoutSubmission(codingAgentCodex, composerOperationPrompt), "fallback preview body",
			func(message messageRecord) string { return fallbackPreviewLine(canonicalPreviewLine(message)) }, "\x1b", "\r"},
	}
	for _, tc := range cases {
		h := newCatalogHost(t, tc.catalog, nil, metadataProfile())
		program := func(message messageRecord) []byte {
			return []byte(tc.prefix + tc.line(message) + tc.submit)
		}
		message, reply := sweepPreview(t, h, tc.body, program)
		if reply.Status != submissionStatusAccepted {
			t.Fatalf("%s: reply %+v", tc.name, reply)
		}
		if got, want := string(lossyUTF8(h.pty.written)), string(lossyUTF8(program(message))); got != want {
			t.Fatalf("%s: written %q, want %q", tc.name, got, want)
		}
	}
}

// queuedReply submits request, which the queue accepts at once, and returns its durable input id.
func queuedReply(t *testing.T, h *inputHost, request inputControlRequest, reason string) int64 {
	t.Helper()
	reply := submitted(t, h.mustHandle(request))
	want := deliveryProjection{Status: deliveryStatusQueued, Method: none[deliveryMethod](), Reason: some(reason)}
	if reply.Status != submissionStatusQueued || reply.Stage != submissionStageZeroBytes || reply.Delivery != some(want) {
		t.Fatalf("reply %+v", reply)
	}
	id, ok := reply.InputID.get()
	if !ok {
		t.Fatal("queued work has a durable input id")
	}
	return id
}

// turnsUntil housekeeps, drains and advances with 5 ms virtual steps until done holds.
func turnsUntil(t *testing.T, h *inputHost, done func() bool) {
	t.Helper()
	for !done() {
		if !h.clock.now().Before(h.until) {
			t.Fatalf("queued work resolves in time; offers %q; screen %q", h.pty.offers, h.fixture.transport.snapshot().screen.Contents())
		}
		housekeep(t, h)
		h.drain()
		h.advance()
		h.clock.advance(5 * time.Millisecond)
	}
}

func inputRow(t *testing.T, h *inputHost, inputID int64) inputOperation {
	t.Helper()
	return mustGetInput(t, h.conn(), inputID)
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_QueuedPromptOverTheLimitSettlesNotSubmittedAndAdvances(t *testing.T) {
	t.Parallel()
	h := newCatalogHost(t, permissionConfiguredCatalog(t, "plain", limitOf(1)), encodedLiteral(), metadataProfile())
	stableForm(t, h)
	id := queuedReply(t, h, literalRequest(h, "prompt:queued-over-limit", ordinaryLiteral), "interaction_open")
	h.show([]byte(ordinaryScreen))
	turnsUntil(t, h, func() bool { return inputRow(t, h, id).state == inputStateSettled })
	status := h.fixture.controller.status()
	input := inputRow(t, h, id)
	if status.visibility != visibilityNotVisible || status.eligibility != (inputEligibilityComposerEmpty{}) || h.fixture.controller.activeEpisodeSeq() != none[int64]() {
		t.Fatalf("status %+v", status)
	}
	if input.disposition != some(inputDispositionNotSubmitted) || input.deliveryReason != some("plain_limit_exceeded") || len(h.pty.offers) != 0 || len(h.coordinator.queue) != 0 {
		t.Fatalf("input %+v", input)
	}
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_SubmitKeyRequiresTheFullVerificationWindowBeforeWriting(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	startSubmission(t, h, "prompt:submit-budget", ordinaryLiteral)
	untilWritten(t, h, len(ordinaryLiteral))
	h.coordinator.pending.deadline = h.clock.now().Add(time.Second)
	assertUncertainReply(t, h, finishSubmission(t, h, false), stopCauseDeadlineExpired)
	if slices.ContainsFunc(h.pty.offers, func(offer []byte) bool { return bytes.Equal(offer, []byte("~")) }) {
		t.Fatal("no submit key without the verification window")
	}
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_ShortImmediateBudgetQueuesZeroBytesThenUsesAFreshRunnerBudget(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	reply, failure := h.handleUntil(literalRequest(h, "prompt:initial-budget", ordinaryLiteral), h.clock.now().Add(time.Second))
	if failure != nil || reply == none[inputControlReply]() {
		t.Fatalf("a short budget queues immediately: %v %v", reply, failure)
	}
	result := submitted(t, reply)
	if result.Status != submissionStatusQueued || result.Stage != submissionStageZeroBytes || len(h.pty.offers) != 0 || replyInput(t, h, result).deferReason != some("delivery_budget_exhausted") {
		t.Fatalf("result %+v", result)
	}
	id, _ := result.InputID.get()
	for range 256 {
		if inputRow(t, h, id).state == inputStateAccepted {
			break
		}
		if len(h.pty.program) != 0 {
			h.childOutput([]byte(ordinaryScreen))
		}
		turn(t, h)
	}
	if inputRow(t, h, id).state != inputStateAccepted {
		t.Fatalf("state %v", inputRow(t, h, id).state)
	}
	assertBytes(t, h.pty.written, encodedLiteral(), "written")
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_DeferredHeadBudgetCheckIncludesTheVerificationWindow(t *testing.T) {
	t.Parallel()
	h := newCatalogHost(t, permissionConfiguredCatalog(t, "plain", nil), encodedLiteral(), metadataProfile())
	stableForm(t, h)
	id := queuedReply(t, h, literalRequest(h, "prompt:deferred-budget", ordinaryLiteral), "interaction_open")
	h.show([]byte(ordinaryScreen))
	if len(h.coordinator.queue) == 0 {
		t.Fatal("the actual queued head")
	}
	h.coordinator.queue[0].attemptDeadline = some(h.clock.now().Add(time.Second))
	turn(t, h)
	if inputRow(t, h, id).state != inputStateReady || len(h.pty.offers) != 0 || len(h.coordinator.queue) == 0 || h.coordinator.queue[0].obstacle != "delivery_budget_exhausted" {
		t.Fatalf("queue %+v", h.coordinator.queue)
	}
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_AnExpiredPostSubmitVerificationRecordsUncertainSubmitUnconfirmed(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	startSubmission(t, h, "prompt:unconfirmed", ordinaryLiteral)
	reply := finishSubmission(t, h, false)
	assertUncertainReply(t, h, reply, stopCauseSubmitUnconfirmed)
	if reply.Stage != submissionStageSubmitted {
		t.Fatalf("stage %v", reply.Stage)
	}
	assertBytes(t, h.pty.written, encodedLiteral(), "written")
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_FreshlyVerifiedEmptyComposerConfirmsTheWrittenSubmitKey(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	startSubmission(t, h, "prompt:confirmed", ordinaryLiteral)
	reply := finishSubmission(t, h, true)
	if reply.Status != submissionStatusAccepted || replyInput(t, h, reply).stopCause != none[stopCause]() {
		t.Fatalf("reply %+v", reply)
	}
	assertBytes(t, h.pty.written, encodedLiteral(), "written")
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_ANewlyOpenedInteractionConfirmsTheWrittenSubmitKey(t *testing.T) {
	t.Parallel()
	h := newCatalogHost(t, permissionConfiguredCatalog(t, "plain", nil), encodedLiteral(), metadataProfile())
	startSubmission(t, h, "prompt:interaction-confirmation", ordinaryLiteral)
	untilWritten(t, h, len(encodedLiteral()))
	stableForm(t, h)
	for range 64 {
		if reply, ok := turn(t, h).get(); ok {
			if reply.Status != submissionStatusAccepted || replyInput(t, h, reply).stopCause != none[stopCause]() {
				t.Fatalf("reply %+v", reply)
			}
			return
		}
	}
	t.Fatal("the new interaction confirms the submitted input")
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_PartialSubmitKeyIsUncertainWithWriteFailed(t *testing.T) {
	t.Parallel()
	definition := inputCatalogDefinition()
	for _, operation := range []string{"prompt", "exec"} {
		plan := jget(definition, "codex", "profile", "capabilities", "composer", "submission", operation)
		jset(plan, "plain", "literal_framing")
		jset(plan, nil, "plain_literal_limit")
	}
	jset(definition, "6b6c", "codex", "profile", "capabilities", "composer", "submission", "prompt", "submit_hex")
	h := newCatalogHost(t, mustLoadCatalog(t, definition), append([]byte(ordinaryLiteral), "kl"...), metadataProfile())
	startSubmission(t, h, "prompt:short-submit", ordinaryLiteral)
	untilWritten(t, h, len(ordinaryLiteral))
	h.pty.short = some(1)
	reply := finishSubmission(t, h, false)
	if reply.Status != submissionStatusUncertain || replyInput(t, h, reply).stopCause != some(stopCauseWriteFailed) {
		t.Fatalf("reply %+v", reply)
	}
}

// failAfterWrite fails the first AfterWrite checkpoint once armed, after recording it.
type failAfterWrite struct {
	inner *inputTestHooks
	armed atomic.Bool
}

func (f *failAfterWrite) checkpoint(boundary inputBoundary, state inputCheckpoint, deadline time.Time) error {
	if err := f.inner.checkpoint(boundary, state, deadline); err != nil {
		return err
	}
	if boundary == inputBoundaryAfterWrite && f.armed.Swap(false) {
		return runtimeError("injected post-write checkpoint failure")
	}
	return nil
}

func finishFault(t *testing.T, h *inputHost, inputID int64) *inputFailure {
	t.Helper()
	for range 64 {
		h.drain()
		reply, failure := h.tryAdvance()
		if failure != nil {
			return failure
		}
		if reply != none[inputControlReply]() {
			t.Fatal("a failed write preserves its primary inputFailure")
		}
		if !slices.ContainsFunc(h.pty.offers, func(offer []byte) bool { return bytes.Equal(offer, pasteCloseBytes) }) && inputRow(t, h, inputID).state != inputStateInProgress {
			t.Fatal("the receipt stays in progress until the paste closes")
		}
	}
	t.Fatal("paste cleanup finishes within bounded drained turns")
	return nil
}

func assertFaultClosed(t *testing.T, h *inputHost, inputID int64) {
	t.Helper()
	if !bytes.Equal(h.pty.offers[len(h.pty.offers)-1], pasteCloseBytes) || slices.ContainsFunc(h.pty.offers, func(offer []byte) bool { return bytes.Equal(offer, []byte("~")) }) ||
		len(h.pty.program) != 0 || len(h.pty.manual) != 0 {
		t.Fatalf("offers %q", h.pty.offers)
	}
	input := inputRow(t, h, inputID)
	if input.state != inputStateUncertain || input.stopCause != some(stopCauseWriteFailed) || input.disposition != none[inputDisposition]() {
		t.Fatalf("input %+v", input)
	}
}

func openPasteHost(t *testing.T, key string) (*inputHost, int64) {
	t.Helper()
	h := newCatalogHost(t, configuredCatalog(t, "bracketed_paste", nil), framingProgram(ordinaryLiteral, true), metadataProfile())
	enablePaste(t, h)
	h.pty.short = some(len(pasteOpen) + 3)
	startSubmission(t, h, key, ordinaryLiteral)
	untilWritten(t, h, len(pasteOpen)+3)
	id, ok := h.coordinator.pending.receiptID().get()
	if !ok {
		t.Fatal("the open paste has a receipt")
	}
	return h, id
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_KnownOpenPasteWriteFailureAttemptsCloseAndPreservesThePrimaryFailure(t *testing.T) {
	t.Parallel()
	for _, closeFails := range []bool{false, true} {
		h, id := openPasteHost(t, "prompt:open-paste-write-failure")
		h.pty.short = none[int]()
		h.pty.failNext = some[error](fixtureIOError{kind: syscall.EPIPE, message: "injected one-shot PTY write failure"})
		if closeFails {
			h.pty.failure = some[error](fixtureIOError{kind: syscall.EACCES, message: "injected PTY write failure"})
		}
		failure := finishFault(t, h, id)
		assertFaultClosed(t, h, id)
		if !strings.Contains(failure.source.message(), "injected one-shot PTY write failure") {
			t.Fatalf("failure %v", failure.source)
		}
		expected := slices.Concat(pasteOpenBytes, []byte(ordinaryLiteral)[:3])
		if !closeFails {
			expected = append(expected, pasteCloseBytes...)
		}
		assertBytes(t, h.pty.written, expected, "written")
	}
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_KnownOpenPasteCheckpointFailureClosesBeforeTerminalSettlement(t *testing.T) {
	t.Parallel()
	h, id := openPasteHost(t, "prompt:open-paste-checkpoint-failure")
	hooks := &failAfterWrite{inner: h.hooks}
	hooks.armed.Store(true)
	h.coordinator.services.hooks = hooks
	failure := finishFault(t, h, id)
	assertFaultClosed(t, h, id)
	if !strings.Contains(failure.source.message(), "injected post-write checkpoint failure") || !bytes.HasSuffix(h.pty.written, pasteCloseBytes) {
		t.Fatalf("failure %v written %q", failure.source, h.pty.written)
	}
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_FailedOrPartialCloseStagesRecordWriteFailed(t *testing.T) {
	t.Parallel()
	for _, failed := range []bool{false, true} {
		h := newCatalogHost(t, permissionConfiguredCatalog(t, "bracketed_paste", nil), framingProgram(ordinaryLiteral, true), metadataProfile())
		enablePaste(t, h)
		h.pty.short = some(len(pasteOpen) + 3)
		startSubmission(t, h, "prompt:bad-close", ordinaryLiteral)
		untilWritten(t, h, len(pasteOpen)+3)
		h.pty.short = none[int]()
		stableForm(t, h)
		h.drain()
		if failed {
			h.pty.failure = some[error](fixtureIOError{kind: syscall.EPIPE, message: "injected PTY write failure"})
		} else {
			h.pty.short = some(1)
		}
		for range 64 {
			reply, failure := h.tryAdvance()
			if failure != nil || reply != none[inputControlReply]() {
				break
			}
			h.drain()
		}
		found, err := findInput(context.Background(), h.conn(), h.run().runID, "prompt:bad-close")
		input, ok := found.get()
		if err != nil || !ok || input.state != inputStateUncertain || input.stopCause != some(stopCauseWriteFailed) ||
			!slices.ContainsFunc(h.pty.offers, func(offer []byte) bool { return bytes.Equal(offer, pasteCloseBytes) }) {
			t.Fatalf("failed=%v: input %+v %v", failed, input, err)
		}
	}
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_AStopInsideTheClosingMarkerClosesPasteAndKeepsShortWriteUncertainty(t *testing.T) {
	t.Parallel()
	h := newCatalogHost(t, permissionConfiguredCatalog(t, "bracketed_paste", nil), framingProgram(ordinaryLiteral, true), metadataProfile())
	enablePaste(t, h)
	prefix := len(pasteOpen) + len(ordinaryLiteral) + 3
	h.pty.short = some(prefix)
	startSubmission(t, h, "prompt:closing-marker-prefix", ordinaryLiteral)
	untilWritten(t, h, prefix)
	h.pty.short = none[int]()
	stableForm(t, h)
	reply := finishSubmission(t, h, false)
	if reply.Status != submissionStatusUncertain || replyInput(t, h, reply).stopCause != some(stopCauseShortWrite) || !bytes.Equal(h.pty.offers[len(h.pty.offers)-1], pasteCloseBytes) {
		t.Fatalf("reply %+v offers %q", reply, h.pty.offers)
	}
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_VerifiedFifoAdvancesPastUncertainAndHoldsTheFollowingOrdinaryHead(t *testing.T) {
	t.Parallel()
	h := newCatalogHost(t, permissionConfiguredCatalog(t, "plain", nil), framingProgram("あxx", false), metadataProfile())
	h.pty.short = some(1)
	startSubmission(t, h, "prompt:uncertain-head", "あxx")
	untilWritten(t, h, 1)
	later := queuedReply(t, h, literalRequest(h, "prompt:behind-uncertain", ordinaryLiteral), "earlier_input")
	stableForm(t, h)
	reply := finishSubmission(t, h, false)
	if reply.Status != submissionStatusUncertain || replyInput(t, h, reply).stopCause != some(stopCauseShortWrite) {
		t.Fatalf("reply %+v", reply)
	}
	writes := slices.Clone(h.pty.written)
	for range 64 {
		turn(t, h)
	}
	if inputRow(t, h, later).state != inputStateReady || len(h.coordinator.queue) == 0 || h.coordinator.queue[0].inputID != later {
		t.Fatalf("the FIFO advanced to the ordinary head: %+v", h.coordinator.queue)
	}
	assertBytes(t, h.pty.written, writes, "no later write")
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_ANonverifiedRunKeepsTheFifoStoppedBehindAnUncertainPrompt(t *testing.T) {
	t.Parallel()
	catalog := permissionConfiguredCatalog(t, "plain", nil).withoutSubmission(codingAgentCodex, composerOperationPrompt)
	h := newCatalogHost(t, catalog, encodedLiteral(), metadataProfile())
	h.confirmSubmissions = false
	h.pty.expectProgram(slices.Concat([]byte("\x1b"), []byte(ordinaryLiteral), []byte("\r")))
	if reply := h.mustHandle(literalRequest(h, "prompt:legacy-uncertain", ordinaryLiteral)); reply != none[inputControlReply]() {
		t.Fatalf("reply %v", reply)
	}
	untilWritten(t, h, 1+len(ordinaryLiteral))
	later := queuedReply(t, h, literalRequest(h, "prompt:legacy-fifo", ordinaryLiteral), "earlier_input")
	stableForm(t, h)
	reply := finishSubmission(t, h, false)
	if reply.Status != submissionStatusUncertain || replyInput(t, h, reply).stopCause == none[stopCause]() {
		t.Fatalf("reply %+v", reply)
	}
	written := slices.Clone(h.pty.written)
	for range 64 {
		turn(t, h)
	}
	if inputRow(t, h, later).state != inputStateReady {
		t.Fatalf("state %v", inputRow(t, h, later).state)
	}
	assertBytes(t, h.pty.written, written, "no later write")
}

// replyAfter drives turns, letting virtual time pass so the pre-submit window can close.
func replyAfter(t *testing.T, h *inputHost) submissionResult {
	t.Helper()
	for range 256 {
		if reply, ok := turn(t, h).get(); ok {
			return reply
		}
		h.clock.advance(waitStep)
	}
	t.Fatal("the coordinator replies within bounded turns")
	return submissionResult{}
}

func erased(h *inputHost) bool {
	return slices.ContainsFunc(h.pty.written, func(b byte) bool { return b == 0x08 || b == 0x15 || b == 0x17 || b == 0x7f })
}

func assertMismatchWithoutSubmit(t *testing.T, h *inputHost, reply submissionResult) {
	t.Helper()
	assertUncertainReply(t, h, reply, stopCauseLiteralMismatch)
	if reply.Stage != submissionStagePartial || len(h.pty.program) != 0 || erased(h) {
		t.Fatalf("a mismatch completes no program and erases nothing: %+v", reply)
	}
}

// unrenderedComposer is the child repainting its composer empty, as a native CLI that has not yet
// rendered the written literal shows it.
const unrenderedComposer = "\x1b[23;1H\x1b[2KSynthetic state\n\x1b[2KState end"

// holdFor drives turns for span of virtual time, asserting the pre-submit window holds the write.
func holdFor(t *testing.T, h *inputHost, span time.Duration) {
	t.Helper()
	until := h.clock.now().Add(span)
	for h.clock.now().Before(until) {
		if turn(t, h) != none[submissionResult]() {
			t.Fatal("the pre-submit window is still open")
		}
		h.clock.advance(waitStep)
	}
	if len(h.pty.program) != 0 {
		t.Fatal("no submit key inside the window")
	}
}

func assertAcceptedClean(t *testing.T, h *inputHost, reply submissionResult) {
	t.Helper()
	if reply.Status != submissionStatusAccepted || replyInput(t, h, reply).stopCause != none[stopCause]() {
		t.Fatalf("reply %+v", reply)
	}
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_PreSubmit_AComposerRenderingTheWrittenLiteralContinuesToTheSubmitKey(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	startSubmission(t, h, "prompt:pre-submit-match", ordinaryLiteral)
	assertAcceptedClean(t, h, finishSubmission(t, h, true))
	assertBytes(t, h.pty.written, encodedLiteral(), "written")
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_PreSubmit_ALiteralRenderingWithinThePreSubmitWindowContinuesToTheSubmitKey(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	startSubmission(t, h, "prompt:pre-submit-late-render", ordinaryLiteral)
	untilWritten(t, h, len(ordinaryLiteral))
	h.childOutput([]byte(unrenderedComposer))
	holdFor(t, h, preSubmitWindow/2)
	h.childOutput(syntheticComposerEcho(ordinaryLiteral))
	assertAcceptedClean(t, h, finishSubmission(t, h, true))
	assertBytes(t, h.pty.written, encodedLiteral(), "written")
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_PreSubmit_AnAppendedKeyRecordsLiteralMismatchWhenThePreSubmitWindowCloses(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	startSubmission(t, h, "prompt:pre-submit-window-close", ordinaryLiteral)
	untilWritten(t, h, len(ordinaryLiteral))
	h.typeHuman([]byte("x"))
	opened := h.clock.now()
	holdFor(t, h, preSubmitWindow-5*waitStep)
	reply := replyAfter(t, h)
	if h.clock.now().Before(opened.Add(preSubmitWindow)) {
		t.Fatal("the mismatch is recorded once the window closes")
	}
	assertMismatchWithoutSubmit(t, h, reply)
	assertBytes(t, h.pty.written, []byte(ordinaryLiteral+"x"), "written")
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_PreSubmit_AKeyTypedDuringTheSettleRecordsLiteralMismatchWithoutASubmitKey(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	startSubmission(t, h, "prompt:pre-submit-appended", ordinaryLiteral)
	untilWritten(t, h, len(ordinaryLiteral))
	h.typeHuman([]byte("x"))
	assertMismatchWithoutSubmit(t, h, replyAfter(t, h))
	assertBytes(t, h.pty.written, []byte(ordinaryLiteral+"x"), "the literal and the typed key stay, with no submit key")
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_PreSubmit_ACollapsedLiteralMatchesThroughItsPlaceholderCount(t *testing.T) {
	t.Parallel()
	literal := strings.Repeat("λ", 40)
	for _, tc := range []struct {
		name     string
		typed    opt[string]
		accepted bool
	}{{"the placeholder count", none[string](), true}, {"one extra scalar", some("y"), false}} {
		h := newCatalogHost(t, inputCatalog(t), framingProgram(literal, false), metadataProfile())
		startSubmission(t, h, "prompt:pre-submit-placeholder", literal)
		untilWritten(t, h, len(literal))
		if typed, ok := tc.typed.get(); ok {
			h.typeHuman([]byte(typed))
		}
		if tc.accepted {
			assertAcceptedClean(t, h, finishSubmission(t, h, true))
		} else {
			assertMismatchWithoutSubmit(t, h, replyAfter(t, h))
		}
	}
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_PreSubmit_AnUnextractableComposerRecordsLiteralMismatch(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	startSubmission(t, h, "prompt:pre-submit-unextractable", ordinaryLiteral)
	untilWritten(t, h, len(ordinaryLiteral))
	h.childOutput([]byte("\x1b[2J\x1b[1;1Hunrelated output"))
	assertMismatchWithoutSubmit(t, h, replyAfter(t, h))
	assertBytes(t, h.pty.written, []byte(ordinaryLiteral), "written")
}

func execRequest(h *inputHost, key string) inputControlRequest {
	request := h.request(key)
	request.Operation = submitExecOperation{input: submitInput{OperationKey: key, InputKind: inputKindExec, Literal: ordinaryLiteral}}
	return request
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_PreSubmit_AMismatchedPromptOrExecStaysUncertainAndStopsTheFifo(t *testing.T) {
	t.Parallel()
	for _, exec := range []bool{false, true} {
		program, literalStage := encodedLiteral(), len(ordinaryLiteral)
		if exec {
			program, literalStage = []byte("! "+ordinaryLiteral+"\r"), 2+len(ordinaryLiteral)
		}
		h := newCatalogHost(t, mustLoadCatalog(t, inputCatalogDefinition()), program, metadataProfile())
		h.confirmSubmissions = false
		first := literalRequest(h, "prompt:pre-submit-fifo", ordinaryLiteral)
		if exec {
			first = execRequest(h, "exec:pre-submit-fifo")
		}
		if reply := h.mustHandle(first); reply != none[inputControlReply]() {
			t.Fatalf("reply %v", reply)
		}
		untilWritten(t, h, literalStage)
		later := queuedReply(t, h, literalRequest(h, "prompt:behind-mismatch", ordinaryLiteral), "earlier_input")
		h.typeHuman([]byte("x"))
		assertMismatchWithoutSubmit(t, h, replyAfter(t, h))
		written := slices.Clone(h.pty.written)
		for range 64 {
			turn(t, h)
		}
		if inputRow(t, h, later).state != inputStateReady {
			t.Fatalf("exec=%v: the queued head waits behind the mismatch", exec)
		}
		assertBytes(t, h.pty.written, written, "no later write")
	}
}

// previewResultCause is the cause kind of the preview's receipt-free delivery result, once the
// event writer has persisted it.
func previewResultCause(t *testing.T, h *inputHost, messageID int64) opt[string] {
	t.Helper()
	h.drain()
	for _, row := range tableRows(t, h.conn(), "SELECT payload_json FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_result'", h.run().runID) {
		payload := jsontext.Value(row[0].(string))
		if string(jsonAt(t, payload, "message_id")) == strconv.FormatInt(messageID, 10) {
			kind := jsonAt(t, payload, "cause", "kind")
			if string(kind) == "null" {
				return none[string]()
			}
			return some(decodedString(t, kind))
		}
	}
	t.Fatalf("a delivery result names message %d", messageID)
	return none[string]()
}

func (h *inputHost) preview(message messageRecord) opt[inputControlReply] {
	h.t.Helper()
	reply, failure := h.coordinator.handlePreview(context.Background(), h.conn(), message, h.fixture.controller, h.until)
	if failure != nil {
		h.t.Fatal(failure.asError())
	}
	return reply
}

func TestRuntimeInput_OrdinaryPrompt_ComposerReliability_PreSubmit_AMismatchedPreviewIsTypedAgainOnceTheComposerIsCleared(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		name  string
		clear func(h *inputHost)
	}{
		{"the user clears the composer", func(*inputHost) {}},
		{"a member press clears the composer", func(h *inputHost) { h.typeHuman([]byte("\x15")) }},
	} {
		h := newInputHost(t)
		h.confirmSubmissions = false
		message, err := sendMessage(context.Background(), h.conn(), h.run().directorMemberID, h.run().memberID, "retyped after a mismatch")
		if err != nil {
			t.Fatal(err)
		}
		line := canonicalPreviewLine(message)
		program := []byte(line + "~")
		h.pty.expectProgram(program)
		if h.preview(message) != none[inputControlReply]() {
			t.Fatalf("%s: the preview waits", tc.name)
		}
		untilWritten(t, h, len(line))
		h.typeHuman([]byte("x"))
		reply := replyAfter(t, h)
		if reply.Status != submissionStatusUncertain || reply.Stage != submissionStagePartial || len(h.pty.program) != 0 || previewResultCause(t, h, message.messageID) != some("literal_mismatch") {
			t.Fatalf("%s: reply %+v", tc.name, reply)
		}
		tc.clear(h)
		h.pty.clearComposer()
		h.show([]byte(ordinaryScreen))
		h.pty.expectProgram(program)
		retyped := h.preview(message)
		var result submissionResult
		if retyped != none[inputControlReply]() {
			result = submitted(t, retyped)
		} else {
			result = finishSubmission(t, h, true)
		}
		if result.Status != submissionStatusAccepted || !bytes.HasSuffix(h.pty.written, program) || !slices.EqualFunc(h.pty.program, [][]byte{program}, bytes.Equal) {
			t.Fatalf("%s: retyped %+v program %q", tc.name, result, h.pty.program)
		}
	}
}

func fallbackProgram(prefix string) []byte {
	return []byte("\x1b" + prefix + ordinaryLiteral + "\r")
}

func assertSettledGaps(t *testing.T, h *inputHost, count int) {
	t.Helper()
	gaps := h.pty.stageGaps()
	if len(gaps) != count {
		t.Fatalf("gaps %v", gaps)
	}
	for _, gap := range gaps {
		if gap < h.stageSettle {
			t.Fatalf("each stage follows its %v settle: %v", h.stageSettle, gap)
		}
	}
}

// permissionCatalog is the inspection fixture with the synthetic composer whose exec submission
// is a placeholder.
func permissionCatalog(t testing.TB) *profileCatalog {
	t.Helper()
	return promptPlanCatalog(t, inspectionComposerDefinition())
}

// untilLiteralWritten submits request and drives turns until the literal stage is written.
func untilLiteralWritten(t *testing.T, h *inputHost, request inputControlRequest) {
	t.Helper()
	if reply := h.mustHandle(request); reply != none[inputControlReply]() {
		t.Fatalf("reply %v", reply)
	}
	for len(h.pty.stages) == 0 {
		if !h.clock.now().Before(h.until) {
			t.Fatal("the literal stage is written")
		}
		h.drain()
		if h.advance() != none[inputControlReply]() {
			t.Fatal("no reply before the submit stage")
		}
		h.clock.advance(5 * time.Millisecond)
	}
}

func finishAfterInterruption(t *testing.T, h *inputHost) submissionResult {
	t.Helper()
	for {
		if !h.clock.now().Before(h.until) {
			t.Fatal("the interrupted input replies")
		}
		h.drain()
		if reply := h.advance(); reply != none[inputControlReply]() {
			return submitted(t, reply)
		}
		h.clock.advance(5 * time.Millisecond)
	}
}

func TestRuntimeInput_OrdinaryPrompt_StagedDelivery_StagedFallbackWithoutAnOperationPlanEscapesSettlesTypesSettlesAndEnters(t *testing.T) {
	t.Parallel()
	h := newCatalogHost(t, inputCatalog(t), fallbackProgram("! "), metadataProfile())
	reply := settle(t, h, execRequest(h, "exec:fallback"))
	if reply.Status != submissionStatusAccepted || !slices.EqualFunc(h.pty.stageBytes(), offers("\x1b", "! "+ordinaryLiteral, "\r"), bytes.Equal) {
		t.Fatalf("reply %+v stages %q", reply, h.pty.stageBytes())
	}
	assertSettledGaps(t, h, 2)
	id, _ := reply.InputID.get()
	if method, reason := storedDeliverySelection(t, h, id); method != some("fallback") || reason != some("submission_plan_unavailable") {
		t.Fatalf("selection %v %v", method, reason)
	}
}

func TestRuntimeInput_OrdinaryPrompt_StagedDelivery_StagedRunnerHoldsThePaneLockAcrossTheSettleAndReleasesItAfterSubmission(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	placement := h.placement()
	untilLiteralWritten(t, h, h.request("prompt:locked"))
	if lock, acquired, err := paneLockTryAcquire(h.fixture.url, placement); err != nil || acquired {
		if lock != nil {
			lock.close()
		}
		t.Fatalf("the runner retains the pane lock between its stages: %v %v", acquired, err)
	}
	if reply := finishAfterInterruption(t, h); reply.Status != submissionStatusAccepted {
		t.Fatalf("reply %+v", reply)
	}
	paneLockAcquireReleased(t, h.fixture.url, placement).close()
}

func TestRuntimeInput_OrdinaryPrompt_StagedDelivery_StagedRunnerWritesZeroBytesWhileAnotherWriterHoldsThePaneLock(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	held, acquired, err := paneLockTryAcquire(h.fixture.url, h.placement())
	if err != nil || !acquired {
		t.Fatalf("the pane lock is free before submission: %v %v", acquired, err)
	}
	if reply := h.mustHandle(h.request("prompt:busy-lock")); reply != none[inputControlReply]() {
		t.Fatalf("reply %v", reply)
	}
	busyUntil := h.clock.now().Add(300 * time.Millisecond)
	for h.clock.now().Before(busyUntil) {
		h.drain()
		if h.advance() != none[inputControlReply]() || len(h.pty.offers) != 0 {
			t.Fatal("no byte while the lock is busy")
		}
		h.clock.advance(5 * time.Millisecond)
	}
	held.close()
	if reply := finishAfterInterruption(t, h); reply.Status != submissionStatusAccepted || !slices.EqualFunc(h.pty.program, [][]byte{encodedLiteral()}, bytes.Equal) {
		t.Fatalf("reply %+v program %q", reply, h.pty.program)
	}
}

func expiredDeadline(h *inputHost) time.Time {
	deadline := h.clock.now()
	h.clock.advance(2 * time.Millisecond)
	return deadline
}

func failAdvanceAtAnExpiredDeadline(t *testing.T, h *inputHost) {
	t.Helper()
	h.drain()
	if _, failure := h.coordinator.advance(context.Background(), h.conn(), h.fixture.transport, h.pty, h.fixture.controller, expiredDeadline(h)); failure == nil {
		t.Fatal("a written attempt fails at its expired deadline")
	}
}

func assertRecordedUncertainWriteFailed(t *testing.T, h *inputHost, inputID int64) {
	t.Helper()
	if row := inputRow(t, h, inputID); row.state != inputStateUncertain || row.stopCause != some(stopCauseWriteFailed) {
		t.Fatalf("row %+v", row)
	}
}

func TestRuntimeInput_OrdinaryPrompt_StagedDelivery_ImmediateFailureBookkeeping_AWrittenImmediatePromptFailingInAdvanceAfterItsLiteralRecordsUncertainWriteFailed(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	untilLiteralWritten(t, h, h.request("prompt:d1-literal"))
	id := h.hooks.inputID(t)
	failAdvanceAtAnExpiredDeadline(t, h)
	assertRecordedUncertainWriteFailed(t, h, id)
}

func TestRuntimeInput_OrdinaryPrompt_StagedDelivery_ImmediateFailureBookkeeping_AWrittenImmediatePromptFailingInAdvanceAfterItsSubmitKeyRecordsUncertainWriteFailed(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	h.confirmSubmissions = false
	untilLiteralWritten(t, h, h.request("prompt:d1-submit"))
	id := h.hooks.inputID(t)
	h.clock.advance(h.stageSettle + 50*time.Millisecond)
	for len(h.pty.stageBytes()) < 2 {
		if !h.clock.now().Before(h.until) {
			t.Fatal("the submit stage is written")
		}
		h.drain()
		if h.advance() != none[inputControlReply]() {
			t.Fatal("no reply before verification")
		}
		h.clock.advance(time.Millisecond)
	}
	failAdvanceAtAnExpiredDeadline(t, h)
	assertRecordedUncertainWriteFailed(t, h, id)
}

func TestRuntimeInput_OrdinaryPrompt_StagedDelivery_ImmediateFailureBookkeeping_AWrittenImmediatePromptFailingInItsPreWriteDrainRecordsUncertainWriteFailed(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	untilLiteralWritten(t, h, h.request("prompt:d2-drain"))
	id := h.hooks.inputID(t)
	h.childOutput([]byte("echo"))
	deadline := expiredDeadline(h)
	_, failure := progressWithManual(h.coordinator, h.fixture.transport, func(tr *transport) (inputDrain, error) {
		return tr.quiesceForInput(h.human, h.pty, h.output, h.fixture.controller.observer(h.fixture.writer, h.clock), deadline)
	})
	if failure == nil {
		t.Fatal("the pre-write drain fails at its deadline")
	}
	_, _ = h.coordinator.pendingAttemptFailed(context.Background(), h.conn(), h.fixture.controller, failure) //nolint:errcheck // the recorded row is what this test asserts
	h.coordinator.finishPendingAttempt()
	assertRecordedUncertainWriteFailed(t, h, id)
}

func assertStoppedWith(t *testing.T, h *inputHost, reply submissionResult, cause stopCause) {
	t.Helper()
	row := replyInput(t, h, reply)
	if reply.Status != submissionStatusUncertain || reply.Stage != submissionStagePartial || row.state != inputStateUncertain || row.stopCause != some(cause) ||
		!slices.EqualFunc(h.pty.stageBytes(), offers(ordinaryLiteral), bytes.Equal) {
		t.Fatalf("%v: reply %+v stages %q", cause, reply, h.pty.stageBytes())
	}
}

func claudeHost(t *testing.T, nativeVersion, idleFixture string, expectedProgram []byte) *inputHost {
	t.Helper()
	profile := metadataProfile()
	profile.backend = codingAgentClaude
	profile.nativeVersion = some(nativeVersion)
	profile.size = terminalSize{rows: 36, cols: 120}
	profile.screen = nativeFixtureBytes(bundledScreen(idleFixture))
	return newCatalogHost(t, profileCatalogBundled(), expectedProgram, profile)
}

func TestRuntimeInput_OrdinaryPrompt_StagedDelivery_StopCause_StagedClaudePendingQuestionNoticeDuringTheSettleStopsBeforeTheSubmitKey(t *testing.T) {
	t.Parallel()
	h := claudeHost(t, "2.1.312", "claude-2.1.312-auto-default-decline-continuation-1", []byte("unused-program~"))
	h.pty.expectProgram([]byte(ordinaryLiteral + "\r"))
	untilLiteralWritten(t, h, h.request("prompt:pending-question-notice"))
	h.show(nativeFixtureBytes(bundledScreen("claude-2.1.312-question-pending-notice-1")))
	reply := finishAfterInterruption(t, h)
	cause, ok := replyInput(t, h, reply).stopCause.get()
	if !ok || (cause != stopCauseInteractionCandidate && cause != stopCauseInteractionOpen) {
		t.Fatalf("a stopped stage records its cause: %v", cause)
	}
	assertStoppedWith(t, h, reply, cause)
	if len(h.pty.program) != 0 || slices.ContainsFunc(h.pty.offers, func(offer []byte) bool { return bytes.Contains(offer, []byte{0x1b}) }) {
		t.Fatal("the submit key is never written")
	}
}

func untilArmed(t *testing.T, h *inputHost) {
	t.Helper()
	h.clock.advance(h.stageSettle + 50*time.Millisecond)
	for h.pendingSnapshot().phase != inputPhaseArmed {
		if !h.clock.now().Before(h.until) {
			t.Fatal("the submit stage arms")
		}
		h.drain()
		if h.advance() != none[inputControlReply]() {
			t.Fatal("no reply while arming")
		}
		h.clock.advance(time.Millisecond)
	}
	if !slices.EqualFunc(h.pty.stageBytes(), offers(ordinaryLiteral), bytes.Equal) {
		t.Fatalf("stages %q", h.pty.stageBytes())
	}
}

func candidateForm(t *testing.T, h *inputHost) {
	t.Helper()
	h.show(inspectionFixtureForm("write candidate.txt", "synthetic-only"))
	if eligibility := h.fixture.controller.health(h.fixture.writer).Eligibility; eligibility != (inputEligibilityHold{reason: gateReasonCandidate}) {
		t.Fatalf("eligibility %v", eligibility)
	}
}

func interactionHost(t *testing.T) *inputHost {
	t.Helper()
	return newCatalogHost(t, permissionCatalog(t), encodedLiteral(), metadataProfile())
}

func TestRuntimeInput_OrdinaryPrompt_StagedDelivery_StopCause_OnlyAnAppearingInteractionOrALiteralMismatchStopsAStagedWrite(t *testing.T) {
	t.Parallel()
	openForm := func(t *testing.T, h *inputHost) { stableForm(t, h) }
	typed := func(data string) func(*testing.T, *inputHost) {
		return func(_ *testing.T, h *inputHost) { h.typeHuman([]byte(data)) }
	}
	cases := []struct {
		name  string
		armed bool
		event func(*testing.T, *inputHost)
		cause opt[stopCause]
	}{
		{"an open interaction during the settle", false, openForm, some(stopCauseInteractionOpen)},
		{"an open interaction once the submit stage is armed", true, openForm, some(stopCauseInteractionOpen)},
		{"a candidate interaction during the settle", false, candidateForm, some(stopCauseInteractionCandidate)},
		{"a candidate interaction once the submit stage is armed", true, candidateForm, some(stopCauseInteractionCandidate)},
		{"a keystroke during the settle", false, typed("x"), some(stopCauseLiteralMismatch)},
		{"a keystroke once the submit stage is armed", true, typed("x"), none[stopCause]()},
		{"a lone Escape during the settle", false, typed("\x1b"), none[stopCause]()},
		{"a pointer report during the settle", false, typed("\x1b[<0;10;20M"), none[stopCause]()},
	}
	for _, tc := range cases {
		h := interactionHost(t)
		h.confirmSubmissions = false
		untilLiteralWritten(t, h, h.request("prompt:staged-event"))
		if tc.armed {
			untilArmed(t, h)
		}
		tc.event(t, h)
		reply := finishSubmission(t, h, true)
		row := replyInput(t, h, reply)
		if cause, stops := tc.cause.get(); stops {
			if reply.Status != submissionStatusUncertain || reply.Stage != submissionStagePartial || row.state != inputStateUncertain || row.stopCause != some(cause) ||
				!slices.EqualFunc(h.pty.stageBytes(), offers(ordinaryLiteral), bytes.Equal) {
				t.Fatalf("%s: reply %+v row %+v stages %q", tc.name, reply, row, h.pty.stageBytes())
			}
		} else if reply.Status != submissionStatusAccepted || reply.Stage != submissionStageSubmitted || row.stopCause != none[stopCause]() || len(h.pty.stageBytes()) != 2 {
			t.Fatalf("%s: the submit key follows the literal: %+v", tc.name, reply)
		}
	}
}

func TestRuntimeInput_OrdinaryPrompt_StagedDelivery_StopCause_PostSubmitVerificationDetectsAnUnsubmittedResult(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		name          string
		verifiesEmpty bool
	}{{"a composer that verifies empty after the submit key", true}, {"a composer that never verifies empty", false}} {
		h := interactionHost(t)
		h.confirmSubmissions = false
		untilLiteralWritten(t, h, h.request("prompt:post-submit"))
		reply := finishSubmission(t, h, tc.verifiesEmpty)
		row := replyInput(t, h, reply)
		if reply.Stage != submissionStageSubmitted || len(h.pty.stageBytes()) != 2 {
			t.Fatalf("%s: the submit key is written: %+v", tc.name, reply)
		}
		if tc.verifiesEmpty {
			if reply.Status != submissionStatusAccepted || row.stopCause != none[stopCause]() {
				t.Fatalf("%s: %+v", tc.name, reply)
			}
		} else if reply.Status != submissionStatusUncertain || row.state != inputStateUncertain || row.stopCause != some(stopCauseSubmitUnconfirmed) {
			t.Fatalf("%s: %+v", tc.name, reply)
		}
	}
}

func TestRuntimeInput_OrdinaryPrompt_StagedDelivery_StopCause_StagedTerminalRepliesDuringTheSettleStillWriteTheSubmitKey(t *testing.T) {
	t.Parallel()
	for _, terminalReply := range []string{"\x1b[I", "\x1b[24;80R", "\x1b]11;rgb:0000/0000/0000\x1b\\"} {
		h := newInputHost(t)
		untilLiteralWritten(t, h, h.request("prompt:reply"))
		h.typeHuman([]byte(terminalReply))
		h.drain()
		if len(h.human.consumed) != len(terminalReply) {
			t.Fatal("the reply is read during the settle")
		}
		reply := finishAfterInterruption(t, h)
		if reply.Status != submissionStatusAccepted || reply.Stage != submissionStageSubmitted || !slices.EqualFunc(h.pty.stageBytes(), offers(ordinaryLiteral, "~"), bytes.Equal) {
			t.Fatalf("%q: reply %+v", terminalReply, reply)
		}
		assertBytes(t, h.pty.manual, []byte(terminalReply), "the reply reaches the child unchanged")
		forwarded := slices.IndexFunc(h.pty.offers, func(offer []byte) bool { return bytes.Equal(offer, []byte(terminalReply)) })
		submit := slices.IndexFunc(h.pty.offers, func(offer []byte) bool { return bytes.Equal(offer, []byte("~")) })
		if forwarded < 0 || submit < 0 || forwarded >= submit {
			t.Fatalf("the reply precedes the submit key: %q", h.pty.offers)
		}
		if row := replyInput(t, h, reply); row.state != inputStateAccepted || row.stopCause != none[stopCause]() {
			t.Fatalf("row %+v", row)
		}
	}
}

func TestRuntimeInput_StagedWriteGuards_AFailedStagedAdvanceReleasesTheComposerPaneLock(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	placement := h.placement()
	h.arm("prompt:failed-advance-pane")
	h.typeHuman([]byte("x"))
	unaccountedDrain(t, h)
	_, failure := h.tryAdvance()
	if failure == nil || !strings.Contains(failure.source.message(), "input manual generation changed") {
		t.Fatalf("the stale proof fails the armed advance: %v", failure)
	}
	if h.coordinator.pending == nil || len(h.pty.stages) != 0 {
		t.Fatalf("a failure before any programmatic byte keeps the attempt pending: %v %q", h.coordinator.pending != nil, h.pty.stageBytes())
	}
	lock, acquired, err := paneLockTryAcquire(h.fixture.url, placement)
	if err != nil || !acquired {
		t.Fatalf("a failed staged advance releases the pane lock its pending attempt held: %v %v", acquired, err)
	}
	lock.close()
}

// unaccountedDrain seals a drain proof outside progressWithManual, so the manual input it read
// is not in the coordinator's accounting.
func unaccountedDrain(t *testing.T, h *inputHost) {
	t.Helper()
	for {
		result, err := h.fixture.transport.quiesceForInput(h.human, h.pty, h.output, h.fixture.controller.observer(h.fixture.writer, h.clock), h.until)
		if err != nil {
			t.Fatal(err)
		}
		if result == inputDrainQuiescent {
			break
		}
		h.clock.advance(time.Millisecond)
	}
	if h.fixture.transport.humanGeneration == h.coordinator.manualGeneration {
		t.Fatal("the direct drain read manual input the coordinator did not account")
	}
}

func TestRuntimeInput_StagedWriteGuards_AClosedUnusedGuardConsumesItsAttemptPermit(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	h.arm("prompt:consumed-permit")
	h.drain()
	h.acquireGuard().close()
	h.drain()
	guard, acquired, err := h.coordinator.acquireWriteGuard(context.Background(), h.conn(), h.fixture.transport, h.fixture.controller, h.until)
	if acquired || err == nil || !strings.Contains(err.Error(), "input attempt permit is consumed") {
		if guard != nil {
			guard.close()
		}
		t.Fatalf("a second guard for the same attempt is refused: %v %v", acquired, err)
	}
	if len(h.pty.offers) != 0 {
		t.Fatalf("offers %q", h.pty.offers)
	}
}

func TestRuntimeInput_StagedWriteGuards_ADrainProofOutsideManualAccountingIsRefusedAsStale(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	h.arm("prompt:unaccounted-manual")
	h.typeHuman([]byte("x"))
	unaccountedDrain(t, h)
	guard, acquired, err := h.coordinator.acquireWriteGuard(context.Background(), h.conn(), h.fixture.transport, h.fixture.controller, h.until)
	if acquired || err == nil || !strings.Contains(err.Error(), "input manual generation changed") {
		if guard != nil {
			guard.close()
		}
		t.Fatalf("a proof outside manual accounting is refused: %v %v", acquired, err)
	}
}

// countedTokens fills each request token with its call number, or fails while fail is set.
type countedTokens struct {
	calls atomic.Int64
	fail  atomic.Bool
}

func (c *countedTokens) fill(out *[32]byte) error {
	call := c.calls.Add(1)
	if c.fail.Load() {
		return runtimeError("owned inspection entropy failure")
	}
	for index := range out {
		out[index] = byte(call) //nolint:gosec // a test mints far fewer than 256 tokens
	}
	return nil
}

func withCountedTokens(h *inputHost) *countedTokens {
	tokens := &countedTokens{}
	h.coordinator.services.tokenSource = tokens
	return tokens
}

// withComposer adds the synthetic composer, its text rule and its submission plans to definition.
func withComposer(definition any) any {
	composer := inputCatalogDefinition()
	jset(definition, jget(composer, "codex", "screens", "synthetic-prompt-plan"), "codex", "screens", "synthetic-prompt-plan")
	rules := jget(definition, "codex", "profile", "rules").(*jsonArr)
	rules.items = append(rules.items, jget(composer, "codex", "profile", "rules", 0), syntheticTextRule())
	jset(definition, jget(composer, "codex", "profile", "capabilities", "composer"), "codex", "profile", "capabilities", "composer")
	return definition
}

func definitionHost(t *testing.T, definition any, program []byte, profile hostProfile) (*inputHost, *countedTokens) {
	t.Helper()
	h := newCatalogHost(t, promptPlanCatalog(t, withComposer(definition)), program, profile)
	return h, withCountedTokens(h)
}

func inspectionHost(t *testing.T) (*inputHost, *countedTokens) {
	t.Helper()
	return definitionHost(t, inspectionFixtureDefinition(), []byte("unused-program~"), metadataProfile())
}

func inspectionRequest(h *inputHost, ids requestIDs) inputControlRequest {
	request := h.request("unused-inspection-key")
	request.Operation = permissionInspectOperation{input: permissionInspectInput{EventID: ids.eventID, InteractionID: ids.interactionID, DirectorMemberID: h.run().directorMemberID}}
	return request
}

func inspectRequest(h *inputHost, request inputControlRequest, until time.Time) (permissionInspection, *inputFailure) {
	h.t.Helper()
	input := request.Operation.(permissionInspectOperation).input
	reply, failure := h.coordinator.inspectReceived(context.Background(), h.conn(), input, h.fixture.controller, h.fixture.transport, until)
	if failure != nil {
		return permissionInspection{}, failure
	}
	value, ok := reply.(permissionInspection)
	if !ok {
		h.t.Fatalf("an actual inspection typed reply: %#v", reply)
	}
	return value, nil
}

func inspectIDs(t *testing.T, h *inputHost, ids requestIDs) permissionInspection {
	t.Helper()
	h.drain()
	value, failure := inspectRequest(h, inspectionRequest(h, ids), h.until)
	if failure != nil {
		t.Fatal(failure.asError())
	}
	return value
}

func inspectionToken(t *testing.T, value permissionInspection) string {
	t.Helper()
	token, ok := value.RequestToken.get()
	if !ok || len(token) != 64 || strings.Trim(token, "0123456789abcdef") != "" {
		t.Fatalf("a current matched request token: %v", value.RequestToken)
	}
	return token
}

func inputRows(t *testing.T, h *inputHost) int64 {
	t.Helper()
	return queryInt64(t, h.conn(), "SELECT COUNT(*) FROM member_input_operations")
}

func (i requestIDs) tuple() [4]int64 {
	return [4]int64{i.eventID, i.interactionID, i.messageID, i.episodeSeq}
}

func TestRuntimeInput_PermissionInspectionContracts_ARunWithoutARecordedNativeVersionGetsATestedPermissionAnswer(t *testing.T) {
	t.Parallel()
	profile := metadataProfile()
	profile.nativeVersion = none[string]()
	h, tokens := definitionHost(t, inspectionFixtureDefinition(), []byte("unused-program~"), profile)
	if h.run().nativeVersion != none[string]() || h.run().coverage != coverageTested {
		t.Fatalf("run %+v", h.run())
	}
	value := inspectIDs(t, h, stableForm(t, h))
	if value.NativeVersion != none[string]() || value.AnswerCapability != coverageTested || value.CapabilityReason != none[string]() ||
		!slices.Equal(value.SupportedDecisions, []permissionDecision{permissionDecisionAllow, permissionDecisionDeny}) || tokens.calls.Load() != 1 {
		t.Fatalf("value %+v", value)
	}
	inspectionToken(t, value)
}

func TestRuntimeInput_PermissionInspectionContracts_EventsAndPermissionInspectionCarryMetadataOnlyEvidence(t *testing.T) {
	t.Parallel()
	h, _ := inspectionHost(t)
	ids := stableForm(t, h)
	inspection := inspectIDs(t, h, ids)
	event := readEvent(t, h.conn(), ids.eventID)
	for _, surface := range []struct {
		name     string
		evidence jsontext.Value
	}{{"event", jsonAt(t, event.event.payloadJSON, "evidence")}, {"inspection", inspection.Evidence}} {
		for _, field := range []string{"operation", "choices", "excerpt"} {
			if string(jsonAt(t, surface.evidence, field)) != "null" {
				t.Fatalf("%s %s: %s", surface.name, field, surface.evidence)
			}
		}
		if strings.Contains(string(surface.evidence), "PRIVATE_INSPECTION_OPERATION") {
			t.Fatalf("%s: %s", surface.name, surface.evidence)
		}
	}
}

func TestRuntimeInput_PermissionInspectionContracts_CurrentRequestBindingComparesEveryFieldExceptOutputSeq(t *testing.T) {
	t.Parallel()
	h, _ := inspectionHost(t)
	inspectIDs(t, h, stableForm(t, h))
	if h.coordinator.currentRequest == nil {
		t.Fatal("an inspected request is current")
	}
	current := h.coordinator.currentRequest.request
	inspectionForm(h, "OTHER_INSPECTION_OPERATION", "synthetic-only")
	settledIDs(t, h)
	if h.fixture.controller.currentRequest == nil {
		t.Fatal("the redrawn request is observed")
	}
	other := *h.fixture.controller.currentRequest
	if reflect.DeepEqual(other.semantic, current.observed.semantic) {
		t.Fatal("the redrawn request has other semantics")
	}
	same := current
	if !current.sameRequest(&same) {
		t.Fatal("a request is the same as itself")
	}
	moved := current
	moved.observed.outputSeq++
	if !current.sameRequest(&moved) {
		t.Fatal("output_seq stays unbound")
	}
	mutations := []struct {
		field  string
		mutate func(*currentRequest)
	}{
		{"request_generation", func(r *currentRequest) { r.observed.requestGeneration++ }},
		{"interaction_id", func(r *currentRequest) { r.interactionID++ }},
		{"event_id", func(r *currentRequest) { r.eventID++ }},
		{"message_id", func(r *currentRequest) { r.messageID++ }},
		{"manual_generation", func(r *currentRequest) { r.manualGeneration++ }},
		{"episode_seq", func(r *currentRequest) { r.observed.episodeSeq++ }},
		{"classification", func(r *currentRequest) { r.observed.classification = interactionClassQuestion }},
		{"request_rule_id", func(r *currentRequest) { r.observed.requestRuleID += "x" }},
		{"semantic", func(r *currentRequest) { r.observed.semantic = other.semantic }},
		{"choices", func(r *currentRequest) { r.observed.choices = append(slices.Clone(r.observed.choices), "x") }},
		{"focus", func(r *currentRequest) { r.observed.focus += "x" }},
		{"answer_matches", func(r *currentRequest) {
			r.observed.answerMatches = slices.Clone(r.observed.answerMatches[:len(r.observed.answerMatches)-1])
		}},
	}
	for _, mutation := range mutations {
		changed := current
		mutation.mutate(&changed)
		if current.sameRequest(&changed) {
			t.Fatalf("%s binds the request", mutation.field)
		}
	}
}

func TestRuntimeInput_PermissionInspectionContracts_PermissionInspectionMatchesDurableCurrentRequestAndPreservesTheArmedWriter(t *testing.T) {
	t.Parallel()
	h, tokens := inspectionHost(t)
	inputID := h.arm("prompt:inspection-pending")
	ids := stableForm(t, h)
	pending := h.pendingSnapshot()
	row := mustGetInput(t, h.conn(), inputID)
	retained := h.retainedCheckpoint(attemptKeyReceipt{id: inputID})
	before := inputRows(t, h)
	h.hooks.mu.Lock()
	hooks := len(h.hooks.points)
	h.hooks.mu.Unlock()
	value := inspectIDs(t, h, ids)
	run := h.run()
	if [4]int64{value.EventID, value.InteractionID, value.MessageID, value.EpisodeSeq} != ids.tuple() ||
		[5]int64{value.RunID, value.OwnerEpoch, value.FleetID, value.MemberID, value.DirectorMemberID} != [5]int64{run.runID, run.ownerEpoch, run.fleetID, run.memberID, run.directorMemberID} {
		t.Fatalf("value %+v", value)
	}
	if value.Classification != some(interactionClassPermissionRequest) || value.Visibility != visibilityVisible || value.AnswerCapability != coverageTested ||
		value.AnswerRuleID != none[string]() || !slices.Equal(value.SupportedDecisions, []permissionDecision{permissionDecisionAllow, permissionDecisionDeny}) ||
		value.CapabilityReason != none[string]() {
		t.Fatalf("distinct Allow/Deny rules have no unique common rule: %+v", value)
	}
	assertJSONEqual(t, value.Evidence, h.fixture.controller.currentEvidence(), "evidence")
	if string(jsonAt(t, value.Evidence, "operation")) != "null" {
		t.Fatal("the evidence names no operation")
	}
	var missing []string
	for _, entry := range value.MissingEvidence {
		missing = append(missing, entry.asStr())
	}
	if !slices.Equal(missing, []string{"operation", "arguments", "cwd", "environment", "targets", "effects", "choice_scope"}) {
		t.Fatalf("missing %q", missing)
	}
	first := inspectionToken(t, value)
	noDrain, failure := inspectRequest(h, inspectionRequest(h, ids), h.until)
	if failure != nil || noDrain.RequestToken != none[string]() || len(noDrain.SupportedDecisions) != 0 || noDrain.AnswerCapability != coverageUnknown {
		t.Fatalf("an inspection without a fresh drain carries no authority: %+v %v", noDrain, failure)
	}
	if inspectionToken(t, inspectIDs(t, h, ids)) != first || tokens.calls.Load() != 1 {
		t.Fatal("the token is stable across inspections")
	}
	if h.pendingSnapshot() != pending || mustGetInput(t, h.conn(), inputID) != row {
		t.Fatal("the armed writer is preserved")
	}
	after := h.retainedCheckpoint(attemptKeyReceipt{id: inputID})
	h.hooks.mu.Lock()
	afterHooks := len(h.hooks.points)
	h.hooks.mu.Unlock()
	if after.stage != retained.stage || after.writtenBytes != retained.writtenBytes || after.outputSeq != retained.outputSeq || inputRows(t, h) != before || afterHooks != hooks || len(h.pty.offers) != 0 {
		t.Fatal("inspection has no input effect")
	}
	stored := tableRows(t, h.conn(), "SELECT i.evidence_json,m.text,m.notified_at FROM member_interactions i JOIN messages m ON m.message_id=?1 WHERE i.interaction_id=?2", ids.messageID, ids.interactionID)
	for _, text := range []string{stored[0][0].(string), stored[0][1].(string)} {
		if strings.Contains(text, first) || strings.Contains(text, "PRIVATE_INSPECTION_OPERATION") {
			t.Fatalf("stored %q", text)
		}
	}
	if stored[0][2] != nil {
		t.Fatalf("notified_at %v", stored[0][2])
	}
}

func TestRuntimeInput_PermissionInspectionContracts_PermissionInspectionTokenBindsFocusPlanSetAndIntermediateInvalidations(t *testing.T) {
	t.Parallel()
	h, tokens := inspectionHost(t)
	ids := stableForm(t, h)
	original := inspectIDs(t, h, ids)
	previous := inspectionToken(t, original)
	revision := h.fixture.controller.requestGeneration
	h.childOutput([]byte("\x1b[?25l\x1b[?25h"))
	h.drain()
	redraw := inspectIDs(t, h, ids)
	if redraw.OutputSeq <= original.OutputSeq || h.fixture.controller.requestGeneration != revision || inspectionToken(t, redraw) != previous {
		t.Fatal("a redraw keeps the token")
	}
	if err := h.fixture.transport.resize(h.pty, terminalSize{rows: 24, cols: 81}); err != nil {
		t.Fatal(err)
	}
	if inspectionToken(t, inspectIDs(t, h, ids)) != previous {
		t.Fatal("a resize keeps the token")
	}
	if err := h.coordinator.observeManual(0); err != nil {
		t.Fatal(err)
	}
	if inspectionToken(t, inspectIDs(t, h, ids)) != previous {
		t.Fatal("zero manual bytes keep the token")
	}
	inspectionForm(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-only")
	rebuilt := inspectionToken(t, inspectIDs(t, h, ids))
	if rebuilt == previous {
		t.Fatal("clear/rebuild traverses incompatible observed screens")
	}
	previous = rebuilt
	h.typeHuman([]byte("!"))
	h.drain()
	assertBytes(t, h.pty.manual, []byte("!"), "manual")
	manual := inspectionToken(t, inspectIDs(t, h, ids))
	if manual == previous {
		t.Fatal("manual input changes the token")
	}
	previous = manual
	revision = h.fixture.controller.requestGeneration
	inspectionForm(h, "PRIVATE_INSPECTION_OPERATION", "missing-focus")
	if h.fixture.controller.currentRequest != nil || h.fixture.controller.requestGeneration <= revision {
		t.Fatal("a missing focus invalidates the request")
	}
	inspectionForm(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-only")
	returned := inspectionToken(t, inspectIDs(t, h, ids))
	if returned == previous {
		t.Fatal("A-invalid-A cannot resurrect a token without an intervening inspect")
	}
	previous = returned
	if currentIDs(t, h) != some(ids) {
		t.Fatal("invalidation precedes durable obscurity debounce")
	}
	inspectionForm(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-other")
	focused := inspectionToken(t, inspectIDs(t, h, ids))
	if focused == previous {
		t.Fatal("a focus change changes the token")
	}
	inspectionForm(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-alt")
	alternate := inspectIDs(t, h, ids)
	if inspectionToken(t, alternate) == focused || !slices.Equal(alternate.SupportedDecisions, []permissionDecision{permissionDecisionAllow}) || alternate.AnswerRuleID != some("inspection-alternate-allow") {
		t.Fatalf("alternate %+v", alternate)
	}
	if currentIDs(t, h) != some(ids) {
		t.Fatal("plan/focus changes are independent of semantic episode identity")
	}
	inspectionForm(h, "different semantic request", "synthetic-only")
	if h.fixture.controller.currentRequest != nil {
		t.Fatal("a different request is not current")
	}
	candidate, failure := inspectRequest(h, inspectionRequest(h, ids), h.until)
	if failure != nil || candidate.RequestToken != none[string]() || len(candidate.SupportedDecisions) != 0 {
		t.Fatalf("candidate %+v %v", candidate, failure)
	}
	inspectionForm(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-alt")
	if inspectionToken(t, inspectIDs(t, h, ids)) == inspectionToken(t, alternate) || tokens.calls.Load() != 7 || len(h.pty.program) != 0 {
		t.Fatalf("calls %d", tokens.calls.Load())
	}
}

func TestRuntimeInput_PermissionInspectionContracts_PermissionInspectionRefusesStaleAssociationsAndExpiredBudgetWithoutEffects(t *testing.T) {
	t.Parallel()
	h, tokens := inspectionHost(t)
	ids := stableForm(t, h)
	original := inspectionToken(t, inspectIDs(t, h, ids))
	before := inputRows(t, h)
	h.drain()
	if _, failure := inspectRequest(h, inspectionRequest(h, ids), h.clock.now()); failure == nil {
		t.Fatal("an expired budget is refused")
	}
	if _, err := h.conn().execute("UPDATE member_interactions SET closed_at=?1,close_reason='not_visible' WHERE interaction_id=?2", nowStamp(), ids.interactionID); err != nil {
		t.Fatal(err)
	}
	h.drain()
	if _, failure := inspectRequest(h, inspectionRequest(h, ids), h.until); failure == nil {
		t.Fatal("a closed interaction is refused")
	}
	if tokens.calls.Load() != 1 || inputRows(t, h) != before || len(h.pty.offers) != 0 || original == "" {
		t.Fatal("refusals have no effect")
	}
}

func TestRuntimeInput_PermissionInspectionContracts_PermissionInspectionEntropyFailureSuppliesNoFallbackAuthority(t *testing.T) {
	t.Parallel()
	h, tokens := inspectionHost(t)
	ids := stableForm(t, h)
	tokens.fail.Store(true)
	h.drain()
	_, failure := inspectRequest(h, inspectionRequest(h, ids), h.until)
	if failure == nil || !strings.Contains(failure.source.message(), "owned inspection entropy failure") || tokens.calls.Load() != 1 {
		t.Fatalf("entropy failure is explicit: %v", failure)
	}
	tokens.fail.Store(false)
	if token := inspectionToken(t, inspectIDs(t, h, ids)); token != strings.Repeat("02", 32) || inputRows(t, h) != 0 || len(h.pty.offers) != 0 {
		t.Fatalf("token %s", token)
	}
}

func TestRuntimeInput_PermissionInspectionContracts_PermissionInspectionOwnerLossRevokesTheExistingToken(t *testing.T) {
	t.Parallel()
	h, tokens := inspectionHost(t)
	ids := stableForm(t, h)
	original := inspectionToken(t, inspectIDs(t, h, ids))
	paths := pathsFor(t, h.run().databaseIdentityJSON, h.run().runID)
	backup := strings.TrimSuffix(paths.ownerLock, filepath.Ext(paths.ownerLock)) + ".inspection-owned-backup"
	h.drain()
	mustRename(t, paths.ownerLock, backup)
	_, lost := inspectRequest(h, inspectionRequest(h, ids), h.until)
	mustRename(t, backup, paths.ownerLock)
	if lost == nil {
		t.Fatal("the actual lock inode no longer occupies its pinned path")
	}
	if inspectionToken(t, inspectIDs(t, h, ids)) == original || tokens.calls.Load() != 2 || len(h.pty.offers) != 0 {
		t.Fatal("owner loss revokes the token")
	}
}

func TestRuntimeInput_PermissionInspectionContracts_PermissionInspectionObservationGapRevokesAuthorityBeforeRecovery(t *testing.T) {
	t.Parallel()
	h, _ := inspectionHost(t)
	ids := stableForm(t, h)
	original := inspectionToken(t, inspectIDs(t, h, ids))
	generation := h.fixture.controller.requestGeneration
	if err := h.fixture.controller.observerFailed(observationFailureParser, h.clock); err != nil {
		t.Fatal(err)
	}
	if h.fixture.controller.currentRequest != nil || h.fixture.controller.requestGeneration <= generation {
		t.Fatal("a gap revokes the current request")
	}
	if value, failure := inspectRequest(h, inspectionRequest(h, ids), h.until); failure == nil && value.RequestToken != none[string]() {
		t.Fatal("no token during the gap")
	}
	later := stableForm(t, h)
	if inspectionToken(t, inspectIDs(t, h, later)) == original || len(h.pty.offers) != 0 {
		t.Fatal("recovery issues a new token")
	}
}

func TestRuntimeInput_PermissionInspectionContracts_PermissionInspectionCurrentAuthorityHasNoDebugOrSerializedRepresentation(t *testing.T) {
	t.Parallel()
	request := currentRequest{observed: observedRequest{focus: "private-focus"}, interactionID: 7}
	assertRedacted(t, "currentRequest", request)
	assertRedacted(t, "*currentRequest", &request)
}

func respondCommand(t *testing.T, h *inputHost, inspection permissionInspection, decision permissionDecision) permissionRespondInput {
	t.Helper()
	return keyedCommand(t, h, inspection, answerKeyDecision{decision: decision})
}

func keyedCommand(t *testing.T, h *inputHost, inspection permissionInspection, key answerKey) permissionRespondInput {
	t.Helper()
	token, ok := inspection.RequestToken.get()
	if !ok {
		t.Fatal("an actual inspected token")
	}
	run := h.run()
	return permissionRespondInput{
		EventID: inspection.EventID, InteractionID: inspection.InteractionID, DirectorMemberID: run.directorMemberID,
		ExpectedRunID: run.runID, ExpectedOwnerEpoch: run.ownerEpoch, RequestToken: token, Key: key,
	}
}

func permissionResponse(t *testing.T, reply opt[inputControlReply]) permissionResponseResult {
	t.Helper()
	value, ok := reply.get()
	if !ok {
		t.Fatal("a permission reply")
	}
	result, ok := value.(permissionResponseResult)
	if !ok {
		t.Fatalf("a typed actual permission response: %#v", value)
	}
	return result
}

// armStage drives turns until the selected answer stage reaches its before-th BeforeWrite
// checkpoint, asserting no byte is offered while it is acquired.
func armStage(t *testing.T, h *inputHost, before int) attemptKey {
	t.Helper()
	offers := len(h.pty.offers)
	for range 16 {
		h.drain()
		if h.advance() != none[inputControlReply]() {
			t.Fatal("stage acquisition yields after its bounded hooks")
		}
		if len(h.pty.offers) != offers {
			t.Fatal("fresh guard acquisition precedes its syscall")
		}
		if h.hooks.count("before") == before {
			return h.hooks.attempt(t)
		}
	}
	t.Fatal("the real selected stage reaches Armed within its finite turn budget")
	return nil
}

const unavailableRequest = "current executable permission request unavailable"

func blockedReason(t *testing.T, h *inputHost, ids requestIDs) string {
	t.Helper()
	inspected, failure := inspectRequest(h, inspectionRequest(h, ids), h.until)
	if failure != nil {
		t.Fatalf("a blocked request is an inspection result, not an error: %v", failure.asError())
	}
	reason, named := inspected.CapabilityReason.get()
	if inspected.RequestToken != none[string]() || len(inspected.SupportedDecisions) != 0 || inspected.AnswerCapability != coverageUnknown || !named {
		t.Fatalf("a blocked request names its reason: %+v", inspected)
	}
	return reason
}

func refusedReason(t *testing.T, h *inputHost, input permissionRespondInput) string {
	t.Helper()
	_, failure := h.respond(input)
	if failure == nil {
		t.Fatal("a blocked request refuses the response")
	}
	return failure.source.message()
}

func admittedResponse(t *testing.T, h *inputHost, input permissionRespondInput) {
	t.Helper()
	reply, failure := h.respond(input)
	if failure != nil || reply != none[inputControlReply]() {
		t.Fatalf("the response is admitted: %v %v", reply, failure)
	}
}

func TestRuntimeInput_PermissionResponseContracts_PermissionInspectionAndResponseNameTheSameBlockingConjunct(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		block func(*testing.T, *inputHost)
	}{
		{"fenced input", func(t *testing.T, h *inputHost) {
			if _, err := h.coordinator.fence(context.Background(), h.until); err != nil {
				t.Fatal(err)
			}
		}},
		{"failed manual accounting", func(t *testing.T, h *inputHost) {
			h.coordinator.manualGeneration = math.MaxUint64
			if err := h.coordinator.observeManual(1); err == nil || !h.coordinator.manualAccountingFailed {
				t.Fatal("an overflowing manual generation fails accounting")
			}
		}},
	}
	var reasons []string
	for _, tc := range cases {
		h, tokens := inspectionHost(t)
		ids := stableForm(t, h)
		input := respondCommand(t, h, inspectIDs(t, h, ids), permissionDecisionAllow)
		minted := tokens.calls.Load()
		tc.block(t, h)
		h.drain()
		reason := blockedReason(t, h, ids)
		if reason == unavailableRequest {
			t.Fatalf("%s names its own conjunct", tc.name)
		}
		if refused := refusedReason(t, h, input); !strings.Contains(refused, reason) {
			t.Fatalf("%s: respond reports the inspect reason %q, got %q", tc.name, reason, refused)
		}
		if tokens.calls.Load() != minted || inputRows(t, h) != 0 || len(h.pty.program) != 0 {
			t.Fatalf("%s: a blocked inspection draws no request token", tc.name)
		}
		reasons = append(reasons, reason)
	}
	if reasons[0] == reasons[1] {
		t.Fatalf("each conjunct has its own reason: %q", reasons)
	}
}

func TestRuntimeInput_PermissionResponseContracts_PermissionScreenSeqOnlyRedrawSchedulesNoRefreshAndKeepsTheSettledToken(t *testing.T) {
	t.Parallel()
	h, _ := inspectionHost(t)
	ids := stableForm(t, h)
	inspected := inspectIDs(t, h, ids)
	input := respondCommand(t, h, inspected, permissionDecisionAllow)
	outputSeq := h.fixture.transport.snapshot().screenSeq
	locker := fileConn(t, h.fixture.url)
	mustExecBatch(t, locker, "BEGIN IMMEDIATE")
	// Every observed frame keeps identical cells with the cursor at the request end: the observer
	// runs after each control byte, so `[24;1HSelection: ` lands as one chunk.
	h.childOutput([]byte("\x1b[?25l\x1b[0m\x1b[24;1HSelection: \x1b[?25h"))
	drained, failure := progressWithManual(h.coordinator, h.fixture.transport, func(tr *transport) (inputDrain, error) {
		return tr.quiesceForInput(h.human, h.pty, h.output, h.fixture.controller.observer(h.fixture.writer, h.clock), h.until)
	})
	if failure != nil {
		t.Fatal(failure.asError())
	}
	if h.fixture.transport.snapshot().screenSeq <= outputSeq {
		t.Fatal("the redraw produced new output")
	}
	if drained != inputDrainQuiescent || h.fixture.controller.currentRequest == nil {
		t.Fatal("a screen_seq-only redraw needs no write, so it settles while the database is locked")
	}
	whileLocked, failure := inspectRequest(h, inspectionRequest(h, ids), h.until)
	if failure != nil || whileLocked.RequestToken != inspected.RequestToken {
		t.Fatalf("the settled request keeps its token without a database write: %v", failure)
	}
	mustExecBatch(t, locker, "COMMIT")
	h.drain()
	if inspectIDs(t, h, ids).RequestToken != inspected.RequestToken {
		t.Fatal("the settled token is unchanged")
	}
	admittedResponse(t, h, input)
	armStage(t, h, 1)
	if inputRows(t, h) != 0 {
		t.Fatal("an answer stores no input row")
	}
}

// neutralOutputFlood is one CSI sequence longer than the drain's per-tick child budget: it changes
// no cell or cursor and yields only two observed frames, so the drain ends with output arriving.
func neutralOutputFlood(h *inputHost) {
	h.childOutput([]byte("\x1b[" + strings.Repeat("0;", 2200) + "m"))
}

const outputArriving = "terminal output still arriving"

// blinkWrites are the blocked Claude Code 2.1.302 form's status-dot blink writes: each moves the
// cursor home, to the bottom row and back to the form without touching form rows.
func blinkWrites() [][]byte {
	writes := make([][]byte, 11)
	for index := range writes {
		dot := " "
		if index%2 == 0 {
			dot = "●"
		}
		writes[index] = []byte("\x1b[H\r\x1b[9B\x1b[38;5;246m" + dot + "\x1b[39m\x1b[80;1H\x1b[23;2H")
	}
	return writes
}

func claudeInspectionHost(t *testing.T) (*inputHost, *countedTokens) {
	t.Helper()
	h := claudeHost(t, "2.1.302", "claude-2.1.302-idle-composer-1", []byte("unused-program~"))
	return h, withCountedTokens(h)
}

func claudeBlockedForm(t *testing.T, h *inputHost) requestIDs {
	t.Helper()
	h.childOutput(nativeFixtureBytes(bundledScreen("claude-2.1.302-default-permission-1")))
	return settledIDs(t, h)
}

func TestRuntimeInput_PermissionResponseContracts_APromptAndExecDuringDialogAQueueWithZeroBytesAndEachDrainsOnce(t *testing.T) {
	t.Parallel()
	const idle = "claude-2.1.302-auto-idle-composer-1"
	const literal = "queued behind the auto-mode nudge"
	fixture := func(id string) []byte { return nativeFixtureBytes(bundledScreen(id)) }
	for _, tc := range []struct{ version, form string }{
		{"2.1.309", "claude-2.1.309-read-outside-permission-1"},
		{"2.1.312", "claude-2.1.312-auto-default-nudge-1"},
	} {
		h := claudeHost(t, tc.version, idle, []byte("unused-program~"))
		withCountedTokens(h)
		h.childOutput(fixture(tc.form))
		settledIDs(t, h)
		if _, held := h.fixture.controller.health(h.fixture.writer).Eligibility.(inputEligibilityHold); !held {
			t.Fatalf("%s: Dialog A holds input", tc.version)
		}
		prompt, exec := literal+"\r", "! "+literal+"\r"
		program := []byte(prompt + exec)
		h.pty.expectProgram(program)
		h.confirmAfterPrograms([]byte(prompt), []byte(exec))
		promptRequest := h.request("prompt:dialog-a")
		promptRequest.Operation = submitPromptOperation{input: submitInput{OperationKey: "prompt:dialog-a", InputKind: inputKindPrompt, Literal: literal}}
		execRequest := h.request("exec:dialog-a")
		execRequest.Operation = submitExecOperation{input: submitInput{OperationKey: "exec:dialog-a", InputKind: inputKindExec, Literal: literal}}
		queued := []int64{
			queuedReply(t, h, promptRequest, "interaction_open"),
			queuedReply(t, h, execRequest, "interaction_open"),
		}
		for range 20 {
			h.drain()
			h.advance()
		}
		if len(h.pty.offers) != 0 {
			t.Fatalf("%s: no byte, Enter included, reaches Dialog A: %q", tc.version, h.pty.offers)
		}
		h.show(fixture(idle))
		drained := func() bool {
			for _, id := range queued {
				if state := mustGetInput(t, h.conn(), id).state; state != inputStateAccepted && state != inputStateSettled {
					return false
				}
			}
			return true
		}
		for !drained() {
			if !h.clock.now().Before(h.until) {
				t.Fatalf("%s: the queued inputs drain in time", tc.version)
			}
			h.drain()
			h.advance()
			h.clock.advance(5 * time.Millisecond)
		}
		for range 20 {
			h.drain()
			h.advance()
		}
		if !slices.EqualFunc(h.pty.program, [][]byte{program}, bytes.Equal) || len(h.pty.manual) != 0 {
			t.Fatalf("%s: each input is delivered once, in order: %q manual %q", tc.version, h.pty.program, h.pty.manual)
		}
		for _, id := range queued {
			input := mustGetInput(t, h.conn(), id)
			if input.state != inputStateAccepted || input.deliveryMethod != some(deliveryMethodProfile) || input.deferReason != some("interaction_open") {
				t.Fatalf("%s: input %d %+v", tc.version, id, input)
			}
		}
	}
}

func TestRuntimeInput_PermissionResponseContracts_PermissionClaudeBlinkCursorTransitKeepsGenerationAndTokenAndAdmitsRespond(t *testing.T) {
	t.Parallel()
	h, _ := claudeInspectionHost(t)
	ids := claudeBlockedForm(t, h)
	inspected := inspectIDs(t, h, ids)
	inspectionToken(t, inspected)
	if !slices.Equal(inspected.SupportedDecisions, []permissionDecision{permissionDecisionAllow, permissionDecisionDeny}) {
		t.Fatalf("decisions %v", inspected.SupportedDecisions)
	}
	generation := h.fixture.controller.requestGeneration
	for index, write := range blinkWrites() {
		h.childOutput(write)
		h.drain()
		if h.fixture.controller.requestGeneration != generation {
			t.Fatalf("blink write %d is a cursor transit", index)
		}
		if h.fixture.controller.currentRequest == nil {
			t.Fatalf("blink write %d ends on the restored request", index)
		}
		if inspectIDs(t, h, ids).RequestToken != inspected.RequestToken {
			t.Fatalf("blink write %d keeps the token", index)
		}
	}
	admittedResponse(t, h, respondCommand(t, h, inspected, permissionDecisionAllow))
}

func TestRuntimeInput_PermissionResponseContracts_PermissionClaudeTransitThatRewritesAFormRowRevokesTheToken(t *testing.T) {
	t.Parallel()
	blink := blinkWrites()[0]
	landing := []byte("\x1b[9B")
	at := bytes.Index(blink, landing)
	if at < 0 {
		t.Fatal("the blink moves down to its status row")
	}
	ontoForm := slices.Concat(blink[:at], []byte("\x1b[21B"), blink[at+len(landing):])
	h, _ := claudeInspectionHost(t)
	ids := claudeBlockedForm(t, h)
	inspected := inspectIDs(t, h, ids)
	generation := h.fixture.controller.requestGeneration
	h.childOutput(ontoForm)
	h.drain()
	if h.fixture.controller.requestGeneration <= generation {
		t.Fatal("a transit frame with a changed form row invalidates the request")
	}
	if inspectIDs(t, h, ids).RequestToken == inspected.RequestToken {
		t.Fatal("the token is revoked")
	}
	if _, failure := h.respond(respondCommand(t, h, inspected, permissionDecisionAllow)); failure == nil {
		t.Fatal("the revoked token refuses respond")
	}
	if inputRows(t, h) != 0 || len(h.pty.program) != 0 {
		t.Fatal("a refused response has no effect")
	}
}

func TestRuntimeInput_PermissionResponseContracts_PermissionPendingAdmissionDrainRefusesRespondBeforeAnyReceipt(t *testing.T) {
	t.Parallel()
	h, _ := inspectionHost(t)
	ids := stableForm(t, h)
	inspected := inspectIDs(t, h, ids)
	input := respondCommand(t, h, inspected, permissionDecisionAllow)
	neutralOutputFlood(h)
	if h.quiesceOnce() != inputDrainPending {
		t.Fatal("the flood leaves the drain pending")
	}
	_, failure := h.respond(input)
	if failure == nil || !strings.Contains(failure.source.message(), "permission request is blocked: "+outputArriving) {
		t.Fatalf("a Pending admission drain refuses the response: %v", failure)
	}
	if failure.context.runID != some(h.run().runID) || failure.context.interactionID != some(ids.interactionID) || failure.context.inputID != none[int64]() {
		t.Fatalf("context %+v", failure.context)
	}
	if inputRows(t, h) != 0 || len(h.pty.offers) != 0 {
		t.Fatal("a refused response has no effect")
	}
	if inspectIDs(t, h, ids).RequestToken != inspected.RequestToken {
		t.Fatal("the neutral output changed no request state")
	}
	admittedResponse(t, h, input)
}

func stableDetailedForm(t *testing.T, h *inputHost, detail string) requestIDs {
	t.Helper()
	h.childOutput([]byte("\x1b[2J\x1b[18;1HSynthetic permission request\r\nOperation: PRIVATE_INSPECTION_OPERATION\r\n> Allow once\r\n  Deny once\r\nRequest form: synthetic-only\r\nDetail: " + detail + "\r\nSelection: "))
	h.drain()
	return settledIDs(t, h)
}

func TestRuntimeInput_PermissionResponseContracts_PointerReportsRevokePermissionTokensWithoutKeyboardGenerationChanges(t *testing.T) {
	t.Parallel()
	for _, pointer := range []string{"\x1b[<0;10;20M", "\x1b[M !!"} {
		h, _ := inspectionHost(t)
		ids := stableDetailedForm(t, h, "opened")
		input := respondCommand(t, h, inspectIDs(t, h, ids), permissionDecisionAllow)
		keyboard := h.fixture.transport.humanGeneration
		h.typeHuman([]byte(pointer))
		h.drain()
		if h.fixture.transport.humanGeneration != keyboard || h.fixture.transport.pointerGeneration != uint64(len(pointer)) {
			t.Fatalf("%q: generations %d %d", pointer, h.fixture.transport.humanGeneration, h.fixture.transport.pointerGeneration)
		}
		if _, failure := h.respond(input); failure == nil {
			t.Fatalf("%q: a pointer report revokes the token", pointer)
		}
		if inputRows(t, h) != 0 || len(h.pty.program) != 0 {
			t.Fatalf("%q: a refused response has no effect", pointer)
		}
	}
}

func admittedAnswer(t *testing.T, h *inputHost) attemptKey {
	t.Helper()
	ids := stableForm(t, h)
	admittedResponse(t, h, respondCommand(t, h, inspectIDs(t, h, ids), permissionDecisionAllow))
	return armStage(t, h, 1)
}

func firstAnswerStage(t *testing.T, h *inputHost, id attemptKey) {
	t.Helper()
	h.drain()
	if h.advance() != none[inputControlReply]() {
		t.Fatal("the first stage yields")
	}
	if point := h.retainedCheckpoint(id); point.stage != submissionStagePartial || point.writtenBytes != 1 {
		t.Fatalf("point %+v", point)
	}
	if !slices.EqualFunc(h.pty.offers, offers("y"), bytes.Equal) {
		t.Fatalf("offers %q", h.pty.offers)
	}
}

// pendingSyscall advances once through a selected-stage write that fails with errno.
func pendingSyscall(t *testing.T, h *inputHost, errno syscall.Errno) {
	t.Helper()
	h.drain()
	h.pty.failNext = some[error](errno)
	reply, failure := h.coordinator.advance(context.Background(), h.conn(), h.fixture.transport, h.pty, h.fixture.controller, h.until)
	if h.pty.failNext != none[error]() {
		t.Fatal("the real selected-stage PtyIo call consumes this fault")
	}
	if failure != nil || reply != none[inputControlReply]() {
		t.Fatalf("a %v write leaves the stage pending: %v %v", errno, reply, failure)
	}
}

func TestRuntimeInput_PermissionResponseContracts_PermissionResponseLaterPendingPreservesPartialAndRetriesOnlyUntouchedStage(t *testing.T) {
	t.Parallel()
	h, _ := inspectionHost(t)
	id := admittedAnswer(t, h)
	firstAnswerStage(t, h, id)
	original := h.coordinator.pending.deadline
	for index, errno := range []syscall.Errno{syscall.EAGAIN, syscall.EINTR} {
		armStage(t, h, index+2)
		pendingSyscall(t, h, errno)
		if point := h.retainedCheckpoint(id); point.stage != submissionStagePartial || point.writtenBytes != 1 {
			t.Fatalf("%v: point %+v", errno, point)
		}
		if h.coordinator.pending.deadline != original {
			t.Fatalf("%v: the deadline is unchanged", errno)
		}
		offered := len(h.pty.offers)
		if reply, failure := h.tryAdvance(); failure != nil || reply != none[inputControlReply]() || len(h.pty.offers) != offered {
			t.Fatalf("%v: a consumed guard requires a new real drain", errno)
		}
	}
	armStage(t, h, 4)
	h.drain()
	result := permissionResponse(t, h.advance())
	if result.Submission.Status != submissionStatusAccepted || result.Submission.Stage != submissionStageSubmitted {
		t.Fatalf("the last untouched stage completes: %+v", result.Submission)
	}
	if !slices.EqualFunc(h.pty.offers, offers("y", "\r", "\r", "\r"), bytes.Equal) {
		t.Fatalf("offers %q", h.pty.offers)
	}
	var after []uint64
	h.hooks.mu.Lock()
	for _, point := range h.hooks.points {
		if point.boundary == "after" {
			after = append(after, point.count)
		}
	}
	h.hooks.mu.Unlock()
	if !slices.Equal(after, []uint64{1, 1, 1, 2}) {
		t.Fatalf("after %v", after)
	}
}

func TestRuntimeInput_PermissionResponseContracts_PermissionResponseFailureAfterItsFirstStageKeepsItsOneWrittenByte(t *testing.T) {
	t.Parallel()
	h, _ := inspectionHost(t)
	firstAnswerStage(t, h, admittedAnswer(t, h))
	inspectionForm(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-other")
	reply, failure := h.tryAdvance()
	if failure != nil {
		t.Fatal(failure.asError())
	}
	result := permissionResponse(t, reply)
	if result.Submission.Status != submissionStatusUncertain || result.Submission.Stage != submissionStagePartial || !slices.EqualFunc(h.pty.offers, offers("y"), bytes.Equal) {
		t.Fatalf("a changed focus ends the answer uncertain: %+v %q", result.Submission, h.pty.offers)
	}
}

func TestRuntimeInput_PermissionResponseContracts_PermissionResponseFailedLaterStageWriteReportsWriteFailed(t *testing.T) {
	t.Parallel()
	h, _ := inspectionHost(t)
	firstAnswerStage(t, h, admittedAnswer(t, h))
	armStage(t, h, 2)
	h.pty.failure = some[error](syscall.EPIPE)
	h.drain()
	reply, failure := h.tryAdvance()
	if failure != nil {
		t.Fatal(failure.asError())
	}
	result := permissionResponse(t, reply)
	if result.Submission.Status != submissionStatusUncertain || result.Submission.Reason != some("write_failed") {
		t.Fatalf("an actual PTY write failure ends the answer uncertain: %+v", result.Submission)
	}
}

func TestRuntimeInput_PermissionResponseContracts_PermissionResponsePositiveShortLaterStageKeepsCumulativeUncertaintyWithoutTail(t *testing.T) {
	t.Parallel()
	definition := inspectionFixtureDefinition()
	jset(definition, "6f6b", "codex", "profile", "capabilities", "answers", 0, "stages", 1, "bytes_hex")
	h, _ := definitionHost(t, definition, []byte("ok"), metadataProfile())
	firstAnswerStage(t, h, admittedAnswer(t, h))
	armStage(t, h, 2)
	h.pty.short = some(1)
	h.drain()
	result := permissionResponse(t, h.advance())
	if result.Submission.Status != submissionStatusUncertain || result.Submission.Stage != submissionStagePartial || result.Submission.InputID != none[int64]() {
		t.Fatalf("a positive short ends the answer plan: %+v", result.Submission)
	}
	if !slices.EqualFunc(h.pty.offers, offers("y", "ok"), bytes.Equal) {
		t.Fatalf("offers %q", h.pty.offers)
	}
	if reply, failure := h.tryAdvance(); failure != nil || reply != none[inputControlReply]() || len(h.pty.offers) != 2 {
		t.Fatal("no tail follows a short stage")
	}
}

const (
	askIdle    = "claude-2.1.302-auto-idle-composer-1"
	askForm    = "claude-2.1.322-ask-single-tmux-120-1"
	askLiteral = "queued behind the AskUserQuestion form"
)

func openedIDs(t *testing.T, h *inputHost) requestIDs {
	t.Helper()
	for {
		h.drain()
		if ids, ok := currentIDs(t, h).get(); ok {
			return ids
		}
		if !h.clock.now().Before(h.until) {
			t.Fatal("the interaction opens within the fixture deadline")
		}
		h.clock.advance(2 * time.Millisecond)
	}
}

func interactionClose(t *testing.T, h *inputHost, interactionID int64) (string, any) {
	t.Helper()
	rows := tableRows(t, h.conn(), "SELECT classification,close_reason FROM member_interactions WHERE interaction_id=?1", interactionID)
	return rows[0][0].(string), rows[0][1]
}

func TestRuntimeInput_PermissionResponseContracts_AskUserQuestion_AskUserQuestionInspectionListsNoChoiceAndReportsNoTestedAnswer(t *testing.T) {
	t.Parallel()
	h := claudeHost(t, "2.1.322", askIdle, []byte("unused-program~"))
	tokens := withCountedTokens(h)
	h.childOutput(nativeFixtureBytes(bundledScreen(askForm)))
	value := inspectIDs(t, h, openedIDs(t, h))
	if value.Classification != some(interactionClassQuestion) || value.Visibility != visibilityVisible || value.AnswerCapability != coverageUnknown ||
		value.CapabilityReason != some("question has no tested answer") || len(value.SupportedChoices) != 0 || len(value.SupportedDecisions) != 0 ||
		value.RequestToken != none[string]() || value.AnswerRuleID != none[string]() {
		t.Fatalf("value %+v", value)
	}
	if rule := decodedString(t, jsonAt(t, value.Evidence, "rule_id")); rule != "claude-ask-user-question" {
		t.Fatalf("rule %q", rule)
	}
	if tokens.calls.Load() != 0 || inputRows(t, h) != 0 || len(h.pty.offers) != 0 {
		t.Fatal("no request token is minted")
	}
}

func TestRuntimeInput_PermissionResponseContracts_AskUserQuestion_APromptDuringTheFormQueuesWithZeroBytesAndDrainsOnceAfterItCloses(t *testing.T) {
	t.Parallel()
	h := claudeHost(t, "2.1.322", askIdle, []byte("unused-program~"))
	withCountedTokens(h)
	h.childOutput(nativeFixtureBytes(bundledScreen(askForm)))
	interactionID := openedIDs(t, h).interactionID
	if _, held := h.fixture.controller.health(h.fixture.writer).Eligibility.(inputEligibilityHold); !held {
		t.Fatal("the form holds input")
	}
	program := []byte(askLiteral + "\r")
	h.pty.expectProgram(program)
	h.confirmAfterPrograms(program)
	request := h.request("prompt:ask-user-question")
	request.Operation = submitPromptOperation{input: submitInput{OperationKey: "prompt:ask-user-question", InputKind: inputKindPrompt, Literal: askLiteral}}
	id := queuedReply(t, h, request, "interaction_open")
	for range 20 {
		h.drain()
		h.advance()
	}
	if len(h.pty.offers) != 0 {
		t.Fatalf("no byte, Escape or Enter included, reaches the form: %q", h.pty.offers)
	}
	if class, closed := interactionClose(t, h, interactionID); class != "question" || closed != nil {
		t.Fatalf("interaction %s %v", class, closed)
	}
	h.show(nativeFixtureBytes(bundledScreen(askIdle)))
	for state := mustGetInput(t, h.conn(), id).state; state != inputStateAccepted && state != inputStateSettled; state = mustGetInput(t, h.conn(), id).state {
		if !h.clock.now().Before(h.until) {
			t.Fatal("the queued prompt drains in time")
		}
		h.drain()
		h.advance()
		h.clock.advance(5 * time.Millisecond)
	}
	if class, closed := interactionClose(t, h, interactionID); class != "question" || closed != "not_visible" {
		t.Fatalf("interaction %s %v", class, closed)
	}
	for range 20 {
		h.drain()
		h.advance()
	}
	if !slices.EqualFunc(h.pty.program, [][]byte{program}, bytes.Equal) || len(h.pty.manual) != 0 {
		t.Fatalf("the prompt is delivered once: %q manual %q", h.pty.program, h.pty.manual)
	}
	if input := mustGetInput(t, h.conn(), id); input.state != inputStateAccepted || input.deferReason != some("interaction_open") {
		t.Fatalf("input %+v", input)
	}
}

func questionHost(t *testing.T, program string) *inputHost {
	t.Helper()
	h, _ := definitionHost(t, questionFixtureDefinition(), []byte(program), metadataProfile())
	return h
}

func questionIDs(t *testing.T, h *inputHost, focused ...string) requestIDs {
	t.Helper()
	h.show(questionFixtureForm(focused))
	return settledIDs(t, h)
}

func choiceKey(id string) answerKey {
	return answerKeyChoice{choice: id}
}

func assertChoices(t *testing.T, choices []supportedChoice, want string) {
	t.Helper()
	encoded, err := json.Marshal(choices)
	if err != nil {
		t.Fatal(err)
	}
	assertJSONEqual(t, jsontext.Value(encoded), jsontext.Value(want), "supported choices")
}

func TestRuntimeInput_PermissionResponseContracts_Question_QuestionInspectionListsItsTestedChoicesInRuleOrderWithAToken(t *testing.T) {
	t.Parallel()
	h := questionHost(t, "3")
	value := inspectIDs(t, h, questionIDs(t, h, "accept"))
	if value.Classification != some(interactionClassQuestion) || value.Visibility != visibilityVisible || value.AnswerCapability != coverageTested || len(value.SupportedDecisions) != 0 {
		t.Fatalf("value %+v", value)
	}
	assertChoices(t, value.SupportedChoices, `[{"choice_id":"decline","label":"Keep current","scope":"persistent"},{"choice_id":"later","label":"Ask later","scope":"persistent"}]`)
	if value.AnswerRuleID != none[string]() || value.CapabilityReason != none[string]() || len(value.MissingEvidence) != 0 {
		t.Fatalf("distinct choice plans have no unique common rule: %+v", value)
	}
	inspectionToken(t, value)
	if inputRows(t, h) != 0 || len(h.pty.offers) != 0 {
		t.Fatal("inspection has no input effect")
	}
}

func TestRuntimeInput_PermissionResponseContracts_Question_QuestionWithOneTestedPlanNamesItsAnswerRule(t *testing.T) {
	t.Parallel()
	definition := questionFixtureDefinition()
	answers := jget(definition, "codex", "profile", "capabilities", "answers").(*jsonArr)
	answers.items = slices.DeleteFunc(answers.items, func(answer any) bool { return jget(answer, "answer_rule_id") == questionLaterPlan })
	h, _ := definitionHost(t, definition, []byte("\x0e\r"), metadataProfile())
	value := inspectIDs(t, h, questionIDs(t, h, "accept"))
	if value.AnswerRuleID != some(questionDeclinePlan) {
		t.Fatalf("rule %v", value.AnswerRuleID)
	}
	assertChoices(t, value.SupportedChoices, `[{"choice_id":"decline","label":"Keep current","scope":"persistent"}]`)
}

func TestRuntimeInput_PermissionResponseContracts_Question_QuestionWithoutATestedPlanReportsUnknownCapabilityAndNoToken(t *testing.T) {
	t.Parallel()
	h := questionHost(t, "3")
	h.show(questionNoticeForm())
	value := inspectIDs(t, h, openedIDs(t, h))
	if value.Classification != some(interactionClassQuestion) || value.AnswerCapability != coverageUnknown || value.CapabilityReason != some("question has no tested answer") ||
		value.RequestToken != none[string]() || value.AnswerRuleID != none[string]() || len(value.SupportedDecisions) != 0 || len(value.SupportedChoices) != 0 || len(value.MissingEvidence) != 0 {
		t.Fatalf("value %+v", value)
	}
}

func TestRuntimeInput_PermissionResponseContracts_Question_ANonFreshQuestionWithTestedPlansKeepsTheGenericReason(t *testing.T) {
	t.Parallel()
	h := questionHost(t, "3")
	ids := questionIDs(t, h, "accept")
	first := inspectionToken(t, inspectIDs(t, h, ids))
	stale, failure := inspectRequest(h, inspectionRequest(h, ids), h.until)
	if failure != nil {
		t.Fatal(failure.asError())
	}
	if stale.Classification != some(interactionClassQuestion) || stale.Visibility != visibilityVisible || stale.CapabilityReason != some(unavailableRequest) ||
		stale.AnswerCapability != coverageUnknown || stale.RequestToken != none[string]() || stale.AnswerRuleID != none[string]() || len(stale.SupportedChoices) != 0 {
		t.Fatalf("a question with tested plans is reinspected, not unanswerable: %+v", stale)
	}
	reinspected := inspectIDs(t, h, ids)
	if reinspected.RequestToken != some(first) || reinspected.CapabilityReason != none[string]() || len(h.pty.offers) != 0 {
		t.Fatalf("reinspected %+v", reinspected)
	}
}

func TestRuntimeInput_PermissionResponseContracts_Question_AnUngatedChoiceWritesItsStageAndReportsTheChoice(t *testing.T) {
	t.Parallel()
	h := questionHost(t, "3")
	inspected := inspectIDs(t, h, questionIDs(t, h, "accept"))
	admittedResponse(t, h, keyedCommand(t, h, inspected, choiceKey("later")))
	armStage(t, h, 1)
	h.drain()
	result := permissionResponse(t, h.advance())
	if result.Submission.Status != submissionStatusAccepted || result.Submission.Stage != submissionStageSubmitted || result.Submission.InputID != none[int64]() || result.Key != choiceKey("later") {
		t.Fatalf("the single choice stage completes: %+v", result)
	}
	if !slices.EqualFunc(h.pty.offers, offers("3"), bytes.Equal) {
		t.Fatalf("offers %q", h.pty.offers)
	}
}

func TestRuntimeInput_PermissionResponseContracts_Question_ASecondAnswerForTheSameEpisodeIsRefusedWithZeroBytes(t *testing.T) {
	t.Parallel()
	h := questionHost(t, "3")
	inspected := inspectIDs(t, h, questionIDs(t, h, "accept"))
	admittedResponse(t, h, keyedCommand(t, h, inspected, choiceKey("later")))
	armStage(t, h, 1)
	h.drain()
	if answered := permissionResponse(t, h.advance()); answered.Submission.Status != submissionStatusAccepted {
		t.Fatalf("the first answer completes: %+v", answered)
	}
	written := slices.Clone(h.pty.offers)
	var replies []permissionResponseResult
	reply, failure := h.respond(keyedCommand(t, h, inspected, choiceKey("later")))
	switch {
	case failure != nil:
	case reply != none[inputControlReply]():
		replies = append(replies, permissionResponse(t, reply))
	default:
		for range 16 {
			h.drain()
			next, failure := h.tryAdvance()
			if failure != nil {
				break
			}
			if next != none[inputControlReply]() {
				replies = append(replies, permissionResponse(t, next))
				break
			}
		}
	}
	if slices.ContainsFunc(replies, func(r permissionResponseResult) bool { return r.Submission.Status == submissionStatusAccepted }) {
		t.Fatal("the episode accepts one answer")
	}
	if !slices.EqualFunc(h.pty.offers, written, bytes.Equal) {
		t.Fatalf("the second answer writes no byte: %q", h.pty.offers)
	}
}

func TestRuntimeInput_PermissionResponseContracts_Question_AGatedChoiceWritesEachStageOnItsMarkerFocus(t *testing.T) {
	t.Parallel()
	h := questionHost(t, "\x0e\r")
	inspected := inspectIDs(t, h, questionIDs(t, h, "accept"))
	admittedResponse(t, h, keyedCommand(t, h, inspected, choiceKey("decline")))
	armStage(t, h, 1)
	h.drain()
	if h.advance() != none[inputControlReply]() || !slices.EqualFunc(h.pty.offers, offers("\x0e"), bytes.Equal) {
		t.Fatalf("the first choice stage keeps the response open: %q", h.pty.offers)
	}
	h.show(questionFixtureForm([]string{"decline"}))
	armStage(t, h, 2)
	h.drain()
	result := permissionResponse(t, h.advance())
	if result.Submission.Status != submissionStatusAccepted || result.Submission.Stage != submissionStageSubmitted || !slices.EqualFunc(h.pty.offers, offers("\x0e", "\r"), bytes.Equal) {
		t.Fatalf("the second choice stage completes: %+v %q", result.Submission, h.pty.offers)
	}
}

func assertRejectedAnswer(t *testing.T, h *inputHost, input permissionRespondInput, reason string) {
	t.Helper()
	before := inputRows(t, h)
	reply, failure := h.respond(input)
	if failure != nil {
		t.Fatal(failure.asError())
	}
	result := permissionResponse(t, reply)
	if result.Submission.Status != submissionStatusRejected || result.Submission.Stage != submissionStageZeroBytes || result.Submission.InputID != none[int64]() ||
		result.Submission.Reason != some(reason) || result.Key != input.Key {
		t.Fatalf("an answer-kind mismatch replies at once with %q: %+v", reason, result)
	}
	if inputRows(t, h) != before || len(h.pty.offers) != 0 {
		t.Fatal("a rejection reserves nothing and writes no byte")
	}
}

func TestRuntimeInput_PermissionResponseContracts_Question_AChoiceOnAPermissionRequestIsRejectedWithZeroBytes(t *testing.T) {
	t.Parallel()
	h := questionHost(t, "y\r")
	h.show(inspectionFixtureForm("PRIVATE_QUESTION_PROFILE", "synthetic-only"))
	inspected := inspectIDs(t, h, settledIDs(t, h))
	if len(inspected.SupportedChoices) != 0 {
		t.Fatalf("choices %+v", inspected.SupportedChoices)
	}
	assertRejectedAnswer(t, h, keyedCommand(t, h, inspected, choiceKey("decline")), "choice answers apply only to question interactions")
}

func TestRuntimeInput_PermissionResponseContracts_Question_ADecisionOnAQuestionIsRejectedWithZeroBytes(t *testing.T) {
	t.Parallel()
	h := questionHost(t, "3")
	inspected := inspectIDs(t, h, questionIDs(t, h, "accept"))
	for _, decision := range []permissionDecision{permissionDecisionAllow, permissionDecisionDeny} {
		assertRejectedAnswer(t, h, respondCommand(t, h, inspected, decision), "decision answers apply only to permission requests")
	}
}

func TestRuntimeInput_PermissionResponseContracts_Question_AChoiceWithoutATestedPlanIsRejectedWithZeroBytes(t *testing.T) {
	t.Parallel()
	h := questionHost(t, "3")
	inspected := inspectIDs(t, h, questionIDs(t, h, "accept"))
	for _, id := range []string{"accept", "missing"} {
		assertRejectedAnswer(t, h, keyedCommand(t, h, inspected, choiceKey(id)), "choice "+id+" has no tested answer for this question")
	}
}

const (
	gatedIdleComposer = "\x1b[0m\x1b[2J\x1b[23;1HSynthetic state\r\nState end"
	gatedOperation    = "PRIVATE_GATED_OPERATION"
)

// responseOutcome is a pending answer's end: a reply, a failure, or neither while it is open.
type responseOutcome struct {
	result  opt[permissionResponseResult]
	failure *inputFailure
}

func gatedHost(t *testing.T) (*inputHost, *countedTokens) {
	t.Helper()
	return definitionHost(t, gatedFixtureDefinition(), []byte("\r"), metadataProfile())
}

func gatedForm(t *testing.T, h *inputHost, focus string) requestIDs {
	t.Helper()
	h.show(gatedFixtureForm(gatedOperation, focus))
	return settledIDs(t, h)
}

func admitUntil(t *testing.T, h *inputHost, ids requestIDs, decision permissionDecision, deadline time.Time) permissionRespondInput {
	t.Helper()
	input := respondCommand(t, h, inspectIDs(t, h, ids), decision)
	reply, failure := h.respondUntil(input, deadline)
	if failure != nil || reply != none[inputControlReply]() {
		t.Fatalf("the response is admitted: %v %v", reply, failure)
	}
	return input
}

func writeStage(t *testing.T, h *inputHost, before int) attemptKey {
	t.Helper()
	id := armStage(t, h, before)
	h.drain()
	if h.advance() != none[inputControlReply]() {
		t.Fatal("a nonfinal stage keeps the response open")
	}
	if h.hooks.count("after") != before {
		t.Fatalf("after %d", h.hooks.count("after"))
	}
	return id
}

// settleResponse drives the pending response until it ends or until passes.
func settleResponse(t *testing.T, h *inputHost, until time.Time) responseOutcome {
	t.Helper()
	for {
		h.drain()
		reply, failure := h.tryAdvance()
		switch {
		case failure != nil:
			return responseOutcome{failure: failure}
		case reply != none[inputControlReply]():
			return responseOutcome{result: some(permissionResponse(t, reply))}
		case !h.clock.now().Before(until):
			return responseOutcome{}
		}
		h.clock.advance(2 * time.Millisecond)
	}
}

func assertUncertainOutcome(t *testing.T, outcome responseOutcome) {
	t.Helper()
	if outcome.failure != nil {
		if outcome.failure.context.inputID != none[int64]() {
			t.Fatalf("an answer failure names no input: %+v", outcome.failure.context)
		}
		return
	}
	result, ended := outcome.result.get()
	if !ended {
		t.Fatal("the response ends instead of waiting")
	}
	if result.Submission.Status != submissionStatusUncertain {
		t.Fatalf("result %+v", result.Submission)
	}
}

func openedEvents(t *testing.T, h *inputHost) int64 {
	t.Helper()
	return queryInt64(t, h.conn(), "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='interaction_opened'", h.run().runID)
}

func runEventCount(t *testing.T, h *inputHost) int64 {
	t.Helper()
	return queryInt64(t, h.conn(), "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1", h.run().runID)
}

func interactionOpen(t *testing.T, h *inputHost, interactionID int64) bool {
	t.Helper()
	return queryInt64(t, h.conn(), "SELECT closed_at IS NULL FROM member_interactions WHERE interaction_id=?1", interactionID) == 1
}

func soon(h *inputHost, span time.Duration) time.Time {
	return h.clock.now().Add(span)
}

// denyThroughStageTwo writes deny stages one and two on the synthetic form, leaving the response
// open.
func denyThroughStageTwo(t *testing.T, h *inputHost, deadline time.Time) {
	t.Helper()
	admitUntil(t, h, gatedForm(t, h, "allow"), permissionDecisionDeny, deadline)
	writeStage(t, h, 1)
	h.show(gatedFixtureForm(gatedOperation, "deny"))
	writeStage(t, h, 2)
	if !slices.EqualFunc(h.pty.offers, offers("h", "\r"), bytes.Equal) {
		t.Fatalf("offers %q", h.pty.offers)
	}
}

func assertOffers(t *testing.T, h *inputHost, want ...string) {
	t.Helper()
	if !slices.EqualFunc(h.pty.offers, offers(want...), bytes.Equal) {
		t.Fatalf("offers %q, want %q", h.pty.offers, want)
	}
}

func TestRuntimeInput_PermissionResponseContracts_Gated_GatedAllowWritesItsStageOnTheMatchingFocus(t *testing.T) {
	t.Parallel()
	h, _ := gatedHost(t)
	admitUntil(t, h, gatedForm(t, h, "allow"), permissionDecisionAllow, h.until)
	armStage(t, h, 1)
	h.drain()
	result := permissionResponse(t, h.advance())
	if result.Submission.Status != submissionStatusAccepted || result.Submission.Stage != submissionStageSubmitted {
		t.Fatalf("the single gated stage completes: %+v", result.Submission)
	}
	assertOffers(t, h, "\r")
}

func TestRuntimeInput_PermissionResponseContracts_Gated_GatedDenyRebindsEachExpectedTransitionWithoutANewToken(t *testing.T) {
	t.Parallel()
	h, tokens := gatedHost(t)
	ids := gatedForm(t, h, "allow")
	admitUntil(t, h, ids, permissionDecisionDeny, h.until)
	minted := tokens.calls.Load()
	id := writeStage(t, h, 1)
	if point := h.retainedCheckpoint(id); point.stage != submissionStagePartial || point.writtenBytes != 1 {
		t.Fatalf("point %+v", point)
	}
	h.show(gatedFixtureForm(gatedOperation, "deny"))
	writeStage(t, h, 2)
	assertOffers(t, h, "h", "\r")
	before := runEventCount(t, h)
	h.show(gatedReasonDialog(none[string]()))
	if runEventCount(t, h) != before || !interactionOpen(t, h, ids.interactionID) {
		t.Fatal("the continuation emits no event and keeps the interaction")
	}
	armStage(t, h, 3)
	h.drain()
	result := permissionResponse(t, h.advance())
	if result.Submission.Status != submissionStatusAccepted || result.Submission.Stage != submissionStageSubmitted {
		t.Fatalf("the continuation stage completes: %+v", result.Submission)
	}
	assertOffers(t, h, "h", "\r", "\r")
	if tokens.calls.Load() != minted || openedEvents(t, h) != 1 {
		t.Fatal("rebinding between stages mints no request token and stays in the original episode")
	}
}

func TestRuntimeInput_PermissionResponseContracts_Gated_GatedFirstStageIsNeverWrittenWhileItsFocusGateIsUnmet(t *testing.T) {
	t.Parallel()
	h, _ := gatedHost(t)
	ids := gatedForm(t, h, "deny")
	deadline := soon(h, 600*time.Millisecond)
	input := respondCommand(t, h, inspectIDs(t, h, ids), permissionDecisionAllow)
	var outcome responseOutcome
	reply, failure := h.respondUntil(input, deadline)
	switch {
	case failure != nil:
		outcome = responseOutcome{failure: failure}
	case reply != none[inputControlReply]():
		outcome = responseOutcome{result: some(permissionResponse(t, reply))}
	default:
		outcome = settleResponse(t, h, deadline.Add(400*time.Millisecond))
	}
	if len(h.pty.offers) != 0 || len(h.pty.program) != 0 {
		t.Fatalf("an unmet gate writes no byte: %q", h.pty.offers)
	}
	if result, ended := outcome.result.get(); ended && result.Submission.Status == submissionStatusAccepted {
		t.Fatal("an unmet gate is never accepted")
	}
	if inputRows(t, h) != 0 {
		t.Fatal("an answer stores no input row")
	}
}

func TestRuntimeInput_PermissionResponseContracts_Gated_GatedStageWaitsWhileThePreviousFocusPersistsUntilTheDeadline(t *testing.T) {
	t.Parallel()
	h, _ := gatedHost(t)
	deadline := soon(h, 900*time.Millisecond)
	admitUntil(t, h, gatedForm(t, h, "allow"), permissionDecisionDeny, deadline)
	writeStage(t, h, 1)
	if waiting := settleResponse(t, h, deadline.Add(-150*time.Millisecond)); waiting.failure != nil || waiting.result != none[permissionResponseResult]() {
		t.Fatalf("the unchanged previous focus keeps the response waiting: %+v", waiting)
	}
	assertOffers(t, h, "h")
	if h.hooks.count("before") != 1 {
		t.Fatal("no later stage reaches BeforeWrite")
	}
	assertUncertainOutcome(t, settleResponse(t, h, deadline.Add(500*time.Millisecond)))
	assertOffers(t, h, "h")
}

func TestRuntimeInput_PermissionResponseContracts_Gated_GatedUnexpectedChangeBetweenStagesFailsUncertainWithoutAnotherByte(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		name   string
		screen []byte
	}{
		{"focus resolved to no choice", gatedFixtureForm(gatedOperation, "none")},
		{"a different operation", gatedFixtureForm("PRIVATE_OTHER_OPERATION", "deny")},
		{"an unexpected continuation", gatedReasonDialog(none[string]())},
	} {
		h, _ := gatedHost(t)
		deadline := soon(h, 3*time.Second)
		admitUntil(t, h, gatedForm(t, h, "allow"), permissionDecisionDeny, deadline)
		writeStage(t, h, 1)
		h.show(tc.screen)
		assertUncertainOutcome(t, settleResponse(t, h, deadline))
		if !slices.EqualFunc(h.pty.offers, offers("h"), bytes.Equal) {
			t.Fatalf("%s: offers %q", tc.name, h.pty.offers)
		}
	}
}

func TestRuntimeInput_PermissionResponseContracts_Gated_GatedContinuationBindsOnlyBySuccessionInTheSameEpisode(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name    string
		disturb func(*testing.T, *inputHost)
	}{
		{"human input", func(_ *testing.T, h *inputHost) {
			h.typeHuman([]byte("x"))
			h.drain()
		}},
		{"pointer input", func(_ *testing.T, h *inputHost) {
			h.typeHuman([]byte("\x1b[<0;10;20M"))
			h.drain()
		}},
		{"an observation gap", func(t *testing.T, h *inputHost) {
			if err := h.fixture.controller.observerFailed(observationFailureParser, h.clock); err != nil {
				t.Fatal(err)
			}
		}},
		{"the episode closing", func(_ *testing.T, h *inputHost) {
			h.show([]byte(gatedIdleComposer))
			h.clock.advance(300 * time.Millisecond)
			h.drain()
		}},
		{"a different episode", func(_ *testing.T, h *inputHost) {
			h.show(gatedFixtureForm("PRIVATE_OTHER_OPERATION", "deny"))
			h.clock.advance(150 * time.Millisecond)
			h.drain()
		}},
	}
	for _, tc := range cases {
		h, _ := gatedHost(t)
		deadline := soon(h, 2500*time.Millisecond)
		denyThroughStageTwo(t, h, deadline)
		tc.disturb(t, h)
		h.show(gatedReasonDialog(none[string]()))
		assertUncertainOutcome(t, settleResponse(t, h, deadline))
		enters := 0
		for _, offer := range h.pty.offers {
			if bytes.Equal(offer, []byte("\r")) {
				enters++
			}
		}
		if enters != 1 {
			t.Fatalf("%s: the continuation stage is never written: %q", tc.name, h.pty.offers)
		}
	}
}

func TestRuntimeInput_PermissionResponseContracts_Gated_GatedContinuationWithAReasonNeverSatisfiesTheScreenGate(t *testing.T) {
	t.Parallel()
	h, _ := gatedHost(t)
	deadline := soon(h, 1500*time.Millisecond)
	denyThroughStageTwo(t, h, deadline)
	h.show(gatedReasonDialog(some("typed reason")))
	assertUncertainOutcome(t, settleResponse(t, h, deadline.Add(500*time.Millisecond)))
	assertOffers(t, h, "h", "\r")
}

func TestRuntimeInput_PermissionResponseContracts_Gated_GatedInspectionOfAContinuationOffersNoTokenAndKeepsTheEpisode(t *testing.T) {
	t.Parallel()
	h, _ := gatedHost(t)
	ids := gatedForm(t, h, "deny")
	inspectionToken(t, inspectIDs(t, h, ids))
	before := runEventCount(t, h)
	h.show(gatedReasonDialog(none[string]()))
	h.clock.advance(150 * time.Millisecond)
	h.drain()
	if blockedReason(t, h, ids) == "" {
		t.Fatal("a blocked continuation names its reason")
	}
	if runEventCount(t, h) != before || !interactionOpen(t, h, ids.interactionID) || openedEvents(t, h) != 1 || len(h.pty.offers) != 0 {
		t.Fatal("the continuation emits no event and keeps the episode")
	}
}

func TestRuntimeInput_PermissionResponseContracts_Gated_GatedStylingOnlyFocusChangeStartsANewRequestGeneration(t *testing.T) {
	t.Parallel()
	h, _ := gatedHost(t)
	ids := gatedForm(t, h, "allow")
	first := inspectIDs(t, h, ids)
	stale := respondCommand(t, h, first, permissionDecisionDeny)
	generation := h.fixture.controller.requestGeneration
	if moved := gatedForm(t, h, "deny"); moved != ids {
		t.Fatalf("a focus move keeps the episode: %+v", moved)
	}
	if h.fixture.controller.requestGeneration <= generation {
		t.Fatal("a focus move starts a new request generation")
	}
	if second := inspectionToken(t, inspectIDs(t, h, ids)); some(second) == first.RequestToken {
		t.Fatal("the moved request has a new token")
	}
	if _, failure := h.respond(stale); failure == nil {
		t.Fatal("the pre-move token is revoked")
	}
	if openedEvents(t, h) != 1 || len(h.pty.offers) != 0 {
		t.Fatal("no new episode and no byte")
	}
}

func opencodeInspectionHost(t *testing.T) (*inputHost, *countedTokens) {
	t.Helper()
	profile := metadataProfile()
	profile.backend = codingAgentOpencode
	profile.nativeVersion = some("2.1.4")
	profile.size = terminalSize{rows: 36, cols: 120}
	profile.screen = nativeFixtureBytes(bundledScreen("opencode-2.1.4-idle-composer-1"))
	h := newCatalogHost(t, profileCatalogBundled(), []byte("\r"), profile)
	return h, withCountedTokens(h)
}

func bundledFixtureScreen(id string) []byte {
	return nativeFixtureBytes(bundledScreen(id))
}

// opencodeScreen is fixture's screen with the lines of linesFrom.
func opencodeScreen(fixture, linesFrom string) []byte {
	screen := parseTestJSON(fixture)
	jset(screen, jget(parseTestJSON(linesFrom), "lines"), "lines")
	return nativeFixtureBytes(testJSONText(screen))
}

func TestRuntimeInput_PermissionResponseContracts_Gated_OpencodeAllowEntersOnTheFocusedAllowOnce(t *testing.T) {
	t.Parallel()
	h, _ := opencodeInspectionHost(t)
	h.show(bundledFixtureScreen("opencode-2.1.4-allow-pre-stage-1"))
	inspected := inspectIDs(t, h, settledIDs(t, h))
	inspectionToken(t, inspected)
	if !slices.Equal(inspected.SupportedDecisions, []permissionDecision{permissionDecisionAllow, permissionDecisionDeny}) {
		t.Fatalf("decisions %v", inspected.SupportedDecisions)
	}
	admittedResponse(t, h, respondCommand(t, h, inspected, permissionDecisionAllow))
	armStage(t, h, 1)
	h.drain()
	if result := permissionResponse(t, h.advance()); result.Submission.Status != submissionStatusAccepted {
		t.Fatalf("the allow stage completes: %+v", result.Submission)
	}
	assertOffers(t, h, "\r")
}

func TestRuntimeInput_PermissionResponseContracts_Gated_OpencodeDenyMovesTheFocusSelectsRejectAndSubmitsTheEmptyReason(t *testing.T) {
	t.Parallel()
	h, tokens := opencodeInspectionHost(t)
	h.show(bundledFixtureScreen("opencode-2.1.4-deny-pre-stage-1"))
	ids := settledIDs(t, h)
	admittedResponse(t, h, respondCommand(t, h, inspectIDs(t, h, ids), permissionDecisionDeny))
	minted := tokens.calls.Load()
	writeStage(t, h, 1)
	assertOffers(t, h, "h")
	h.show(opencodeScreen(bundledScreen("opencode-2.1.4-deny-pre-stage-2"), bundledScreen("opencode-2.1.4-deny-pre-stage-1")))
	writeStage(t, h, 2)
	assertOffers(t, h, "h", "\r")
	h.show(bundledFixtureScreen("opencode-2.1.4-deny-pre-stage-3"))
	if !interactionOpen(t, h, ids.interactionID) {
		t.Fatal("the reject dialog keeps the interaction open")
	}
	armStage(t, h, 3)
	h.drain()
	result := permissionResponse(t, h.advance())
	if result.Submission.Status != submissionStatusAccepted || result.Submission.Stage != submissionStageSubmitted {
		t.Fatalf("the empty-reason stage completes: %+v", result.Submission)
	}
	assertOffers(t, h, "h", "\r", "\r")
	if tokens.calls.Load() != minted || openedEvents(t, h) != 1 {
		t.Fatal("one token and one episode")
	}
}

func TestRuntimeInput_PermissionResponseContracts_Gated_OpencodeFocusOnAlwaysAllowAfterTheFirstDenyStageFailsUncertain(t *testing.T) {
	t.Parallel()
	h, _ := opencodeInspectionHost(t)
	h.show(bundledFixtureScreen("opencode-2.1.4-deny-pre-stage-1"))
	ids := settledIDs(t, h)
	deadline := soon(h, 3*time.Second)
	admitUntil(t, h, ids, permissionDecisionDeny, deadline)
	writeStage(t, h, 1)
	h.show(opencodeScreen(bundledScreen("opencode-2.1.4-focus-always-1"), bundledScreen("opencode-2.1.4-deny-pre-stage-1")))
	assertUncertainOutcome(t, settleResponse(t, h, deadline))
	assertOffers(t, h, "h")
}

// opencodeCommand is the bundled OpenCode fixture id with the command box row row showing command.
func opencodeCommand(t *testing.T, id string, row int, command string, trailerCol int) []byte {
	t.Helper()
	screen := parseTestJSON(bundledScreen(id))
	cols, err := strconv.Atoi(string(jget(screen, "cols").(jsonNumber)))
	if err != nil {
		t.Fatal(err)
	}
	for offset, text := range []string{command, ""} {
		jset(screen, fmt.Sprintf("%-*s█%s", trailerCol, text, strings.Repeat(" ", cols-trailerCol-1)), "lines", row+offset)
	}
	return nativeFixtureBytes(testJSONText(screen))
}

func TestRuntimeInput_PermissionResponseContracts_Gated_OpencodeResizeAcrossWSplitBetweenDenyStagesFailsUncertain(t *testing.T) {
	t.Parallel()
	const command = "  $ /tmp/x/permission-probe deny"
	h, _ := opencodeInspectionHost(t)
	h.show(opencodeCommand(t, "opencode-2.1.4-deny-pre-stage-1", 14, command, 116))
	ids := settledIDs(t, h)
	deadline := soon(h, 3*time.Second)
	admitUntil(t, h, ids, permissionDecisionDeny, deadline)
	writeStage(t, h, 1)
	generation := h.fixture.controller.requestGeneration
	if err := h.fixture.transport.resize(h.pty, terminalSize{rows: 36, cols: 57}); err != nil {
		t.Fatal(err)
	}
	h.show(opencodeCommand(t, "opencode-2.1.4-narrow-deny-pre-stage-2-057-1", 17, command, 53))
	assertUncertainOutcome(t, settleResponse(t, h, deadline))
	assertOffers(t, h, "h")
	if h.fixture.controller.requestGeneration <= generation {
		t.Fatal("the layout change starts a new request generation")
	}
	if openedEvents(t, h) != 1 {
		t.Fatal("unchanged command rows keep the episode")
	}
}

func TestRuntimeInput_PermissionResponseContracts_Gated_OpencodeRejectDialogReachedByHumanInputOffersNoToken(t *testing.T) {
	t.Parallel()
	h, _ := opencodeInspectionHost(t)
	h.show(bundledFixtureScreen("opencode-2.1.4-deny-pre-stage-1"))
	ids := settledIDs(t, h)
	inspectionToken(t, inspectIDs(t, h, ids))
	h.show(bundledFixtureScreen("opencode-2.1.4-deny-pre-stage-3"))
	h.clock.advance(150 * time.Millisecond)
	h.drain()
	blockedReason(t, h, ids)
	if !interactionOpen(t, h, ids.interactionID) || openedEvents(t, h) != 1 {
		t.Fatal("the reject dialog keeps the episode")
	}
}

func answerLater(t *testing.T, h *inputHost, ids requestIDs) permissionResponseResult {
	t.Helper()
	admittedResponse(t, h, keyedCommand(t, h, inspectIDs(t, h, ids), choiceKey("later")))
	armStage(t, h, h.hooks.count("before")+1)
	h.drain()
	return permissionResponse(t, h.advance())
}

func TestRuntimeInput_PermissionAnswerGuards_AFinishedAnswerFreesItsReceiptCacheSlot(t *testing.T) {
	t.Parallel()
	h := questionHost(t, "3")
	for index := range receiptCacheCapacity - 1 {
		h.coordinator.operations[attemptKeyReceipt{id: -int64(index) - 1}] = &retainedInput{operation: attemptRecordReceipt{operation: inputOperation{state: inputStateAccepted}}}
	}
	if first := answerLater(t, h, questionIDs(t, h, "accept")); first.Submission.Status != submissionStatusAccepted {
		t.Fatalf("the first answer completes: %+v", first.Submission)
	}
	h.show(questionNoticeForm())
	h.clock.advance(300 * time.Millisecond)
	h.drain()
	if second := answerLater(t, h, questionIDs(t, h, "accept")); second.Submission.Status != submissionStatusAccepted {
		t.Fatalf("the next episode's answer fits the cache its predecessor freed: %+v", second.Submission)
	}
	assertOffers(t, h, "3", "3")
}

func TestRuntimeInput_PermissionAnswerGuards_AReissuedTokenForTheSameRequestStopsTheAnswerBetweenStages(t *testing.T) {
	t.Parallel()
	h, _ := inspectionHost(t)
	id := admittedAnswer(t, h)
	firstAnswerStage(t, h, id)
	ids := settledIDs(t, h)
	paths := pathsFor(t, h.run().databaseIdentityJSON, h.run().runID)
	backup := strings.TrimSuffix(paths.ownerLock, filepath.Ext(paths.ownerLock)) + ".inspection-owned-backup"
	h.drain()
	mustRename(t, paths.ownerLock, backup)
	_, lost := inspectRequest(h, inspectionRequest(h, ids), h.until)
	mustRename(t, backup, paths.ownerLock)
	if lost == nil {
		t.Fatal("owner loss revokes the token")
	}
	inspectionToken(t, inspectIDs(t, h, ids))
	assertUncertainOutcome(t, settleResponse(t, h, soon(h, 500*time.Millisecond)))
	assertOffers(t, h, "y")
}

func TestRuntimeInput_PermissionAnswerGuards_ManualOrPointerInputDuringAnAnswerStopsItAsManualInput(t *testing.T) {
	t.Parallel()
	for _, typed := range []string{"x", "\x1b[<0;10;20M"} {
		h, _ := inspectionHost(t)
		firstAnswerStage(t, h, admittedAnswer(t, h))
		h.typeHuman([]byte(typed))
		outcome := settleResponse(t, h, soon(h, 500*time.Millisecond))
		result, ended := outcome.result.get()
		if outcome.failure != nil || !ended || result.Submission.Status != submissionStatusUncertain || result.Submission.Reason != some("manual_input") {
			t.Fatalf("%q: outcome %+v %v", typed, result.Submission, outcome.failure)
		}
	}
}

const busyDraft = "\x1b[2J\x1b[23;1HSynthetic draft\r\nDraft end"

// busyCatalog is the prompt-plan catalog, whose composer the draft screen leaves unverified.
func busyCatalog(t testing.TB) *profileCatalog {
	t.Helper()
	return promptPlanCatalog(t, inputCatalogDefinition())
}

func permissionBusyCatalog(t testing.TB) *profileCatalog {
	t.Helper()
	return promptPlanCatalog(t, withComposer(inspectionFixtureDefinition()))
}

func showBusy(t *testing.T, h *inputHost) {
	t.Helper()
	h.show([]byte(busyDraft))
	if _, held := h.fixture.controller.health(h.fixture.writer).Eligibility.(inputEligibilityHold); !held {
		t.Fatal("a draft holds input")
	}
}

// holdTurns drives a fixed number of runner turns, enough for a misbehaving queue to write.
func holdTurns(h *inputHost) {
	for range 40 {
		h.drain()
		h.advance()
	}
}

// busyHost is a host whose deadline only bounds a hang, so suite load cannot fail a delivery.
func busyHost(t *testing.T, catalog *profileCatalog) *inputHost {
	t.Helper()
	return buildInputHost(t, catalog, encodedLiteral(), metadataProfile(), time.Now().Add(60*time.Second))
}

func delivered(t *testing.T, h *inputHost, inputID int64) bool {
	t.Helper()
	state := inputRow(t, h, inputID).state
	return state == inputStateAccepted || state == inputStateSettled
}

func foundInput(t *testing.T, h *inputHost, key string) inputOperation {
	t.Helper()
	found, err := findInput(context.Background(), h.conn(), h.run().runID, key)
	if err != nil {
		t.Fatal(err)
	}
	input, ok := found.get()
	if !ok {
		t.Fatalf("a durable receipt for %s", key)
	}
	return input
}

func busyThenProfile(t *testing.T, h *inputHost) {
	t.Helper()
	id := queuedReply(t, h, h.request("prompt:busy-composer"), "composer_busy")
	holdTurns(h)
	if len(h.pty.offers) != 0 || inputRow(t, h, id).state != inputStateReady {
		t.Fatalf("a busy composer receives no byte, Escape included: %q", h.pty.offers)
	}
	h.show([]byte(ordinaryScreen))
	turnsUntil(t, h, func() bool { return delivered(t, h, id) })
	if !slices.EqualFunc(h.pty.stageBytes(), offers(ordinaryLiteral, "~"), bytes.Equal) {
		t.Fatalf("stages %q", h.pty.stageBytes())
	}
	if input := inputRow(t, h, id); input.deliveryMethod != some(deliveryMethodProfile) || input.deliveryReason != none[string]() || input.deferReason != some("composer_busy") {
		t.Fatalf("input %+v", input)
	}
}

func TestRuntimeInput_OrdinaryPrompt_ComposerBusy_ComposerBusyQueuesWithZeroBytesAndDrainsByProfileWhenTheComposerEmpties(t *testing.T) {
	t.Parallel()
	h := busyHost(t, busyCatalog(t))
	h.show([]byte(busyDraft))
	if eligibility := h.fixture.controller.health(h.fixture.writer).Eligibility; eligibility != (inputEligibilityHold{reason: gateReasonUnknown}) {
		t.Fatalf("eligibility %v", eligibility)
	}
	busyThenProfile(t, h)
}

func TestRuntimeInput_OrdinaryPrompt_ComposerBusy_ComposerBusySelectionRequeuesWithZeroBytesWhenTheScreenBlocksBeforeTheFirstWrite(t *testing.T) {
	t.Parallel()
	cases := []struct {
		reason  string
		catalog func(testing.TB) *profileCatalog
		block   func(*testing.T, *inputHost)
	}{
		{"interaction_open", permissionCatalog, func(t *testing.T, h *inputHost) { stableForm(t, h) }},
		{"composer_busy", busyCatalog, func(_ *testing.T, h *inputHost) { h.show([]byte(busyDraft)) }},
	}
	for _, tc := range cases {
		h := busyHost(t, tc.catalog(t))
		const key = "prompt:blocked-after-selection"
		if h.mustHandle(h.request(key)) != none[inputControlReply]() {
			t.Fatalf("%s: an eligible composer selects its method immediately", tc.reason)
		}
		tc.block(t, h)
		holdTurns(h)
		if len(h.pty.offers) != 0 {
			t.Fatalf("%s: no byte, Escape included: %q", tc.reason, h.pty.offers)
		}
		requeued := foundInput(t, h, key)
		if requeued.state != inputStateReady || requeued.deferReason != some(tc.reason) || requeued.deliveryMethod != none[deliveryMethod]() {
			t.Fatalf("%s: requeued %+v", tc.reason, requeued)
		}
		h.show([]byte(ordinaryScreen))
		turnsUntil(t, h, func() bool { return delivered(t, h, requeued.inputID) })
		if !slices.EqualFunc(h.pty.program, [][]byte{encodedLiteral()}, bytes.Equal) {
			t.Fatalf("%s: program %q", tc.reason, h.pty.program)
		}
	}
}

func TestRuntimeInput_OrdinaryPrompt_ComposerBusy_ComposerBusyDoesNotHoldAWorkingMemberWhoseComposerRowIsEmpty(t *testing.T) {
	t.Parallel()
	h := busyHost(t, inputCatalog(t))
	h.show([]byte("\x1b[2J\x1b[21;1HWorking\x1b[23;1HSynthetic state\r\nState end"))
	reply := settle(t, h, h.request("prompt:working-empty-row"))
	if reply.Status != submissionStatusAccepted || !slices.EqualFunc(h.pty.stageBytes(), offers(ordinaryLiteral, "~"), bytes.Equal) {
		t.Fatalf("the native CLI queues profile input behind its current turn: %+v %q", reply, h.pty.stageBytes())
	}
	if input := replyInput(t, h, reply); input.deliveryMethod != some(deliveryMethodProfile) || input.deferredAt != none[string]() {
		t.Fatalf("input %+v", input)
	}
}

func TestRuntimeInput_OrdinaryPrompt_ComposerBusy_ComposerBusyRequeuesAStartedDeferredAttemptWithZeroBytesBeforeItsFirstStage(t *testing.T) {
	t.Parallel()
	h := busyHost(t, permissionBusyCatalog(t))
	stableForm(t, h)
	id := queuedReply(t, h, h.request("prompt:busy-after-start"), "interaction_open")
	behind := queuedReply(t, h, h.request("prompt:behind"), "interaction_open")
	h.show([]byte(ordinaryScreen))
	for inputRow(t, h, id).state == inputStateReady {
		if !h.clock.now().Before(h.until) {
			t.Fatal("the deferred attempt starts on the verified empty composer")
		}
		h.drain()
		h.advance()
		h.clock.advance(5 * time.Millisecond)
	}
	if inputRow(t, h, id).state != inputStateInProgress || len(h.pty.offers) != 0 {
		t.Fatal("the attempt starts with zero bytes")
	}
	h.show([]byte(busyDraft))
	holdTurns(h)
	if len(h.pty.offers) != 0 {
		t.Fatalf("a draft before the first stage keeps zero bytes: %q", h.pty.offers)
	}
	if requeued := inputRow(t, h, id); requeued.state != inputStateReady || requeued.disposition != none[inputDisposition]() ||
		requeued.deliveryMethod != none[deliveryMethod]() || requeued.deferReason != some("interaction_open") {
		t.Fatalf("defer_reason keeps why the input first entered the queue: %+v", requeued)
	}
	if inputRow(t, h, behind).state != inputStateReady {
		t.Fatal("the later input waits")
	}
	h.show([]byte(ordinaryScreen))
	turnsUntil(t, h, func() bool { return delivered(t, h, id) })
	if inputRow(t, h, behind).state != inputStateReady {
		t.Fatal("the requeued input keeps its head position")
	}
	if !slices.EqualFunc(h.pty.offers, offers(ordinaryLiteral, "~"), bytes.Equal) || inputRow(t, h, id).deliveryMethod != some(deliveryMethodProfile) {
		t.Fatalf("offers %q", h.pty.offers)
	}
}

func TestRuntimeInput_OrdinaryPrompt_ComposerBusy_ComposerBusyRequeuedImmediateInputKeepsItsPlaceAheadOfLaterQueuedInput(t *testing.T) {
	t.Parallel()
	h := busyHost(t, busyCatalog(t))
	if h.mustHandle(h.request("prompt:first")) != none[inputControlReply]() {
		t.Fatal("the first input selects its method immediately")
	}
	later := queuedReply(t, h, h.request("prompt:later"), "earlier_input")
	showBusy(t, h)
	holdTurns(h)
	if len(h.pty.offers) != 0 {
		t.Fatalf("offers %q", h.pty.offers)
	}
	first := foundInput(t, h, "prompt:first").inputID
	if inputRow(t, h, first).state != inputStateReady {
		t.Fatal("the busy composer requeues the first input")
	}
	h.show([]byte(ordinaryScreen))
	turnsUntil(t, h, func() bool {
		if delivered(t, h, later) && !delivered(t, h, first) {
			t.Fatal("the requeued first input drains before the later one")
		}
		return delivered(t, h, first) && delivered(t, h, later)
	})
}

// sqlObservation counts transaction begins on the supplied connection and, once it sees the
// member run read, optionally lets virtual time pass delay.
type sqlObservation struct {
	begins  atomic.Int64
	runRead atomic.Bool
	entered atomic.Bool
	delay   opt[time.Time]
	clock   *testClock
}

func observeSQL(t *testing.T, conn *dbConn, state *sqlObservation) {
	t.Helper()
	setAuthorizer(t, conn, func(action sqlite3.AuthorizerActionCode, name3rd, _ string) sqlite3.AuthorizerReturnCode {
		switch {
		case action == sqlite3.AUTH_TRANSACTION && name3rd == "BEGIN":
			state.begins.Add(1)
		case action == sqlite3.AUTH_READ && name3rd == "member_runs":
			state.runRead.Store(true)
			if until, delayed := state.delay.get(); delayed && !state.entered.Swap(true) {
				passUntil(state.clock, until)
			}
		}
		return sqlite3.AUTH_OK
	})
	t.Cleanup(func() { clearAuthorizer(t, conn) })
}

func passUntil(clock *testClock, until time.Time) {
	clock.advance(max(until.Sub(clock.now()), 0) + time.Millisecond)
}

func busyTimeoutMillis(t *testing.T, conn *dbConn) int64 {
	t.Helper()
	return queryInt64(t, conn, "PRAGMA busy_timeout")
}

func setBusyTimeout(t *testing.T, conn *dbConn, timeout time.Duration) {
	t.Helper()
	if err := conn.raw.BusyTimeout(timeout); err != nil {
		t.Fatal(err)
	}
}

func TestRuntimeInput_OrdinaryPrompt_Sql_PromptFinalSqlIsNonwaitingOnTheSuppliedConnectionAndFreshDrainProgresses(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	id := h.arm("prompt:contended")
	h.drain()
	setBusyTimeout(t, h.conn(), 600*time.Millisecond)
	blocker := fileConn(t, h.fixture.url)
	mustExecBatch(t, blocker, "BEGIN IMMEDIATE")
	observeSQL(t, h.conn(), &sqlObservation{clock: h.clock})
	started := time.Now()
	pending, err := withDatabaseBudget(context.Background(), h.conn(), deadlineUntil(h.until), func(ctx context.Context, conn *dbConn) (opt[inputControlReply], error) {
		reply, failure := h.coordinator.advance(ctx, conn, h.fixture.transport, h.pty, h.fixture.controller, h.until)
		if failure != nil {
			return reply, failure.source
		}
		if !conn.raw.GetAutocommit() {
			t.Error("the advance leaves no transaction open")
		}
		return reply, nil
	})
	elapsed := time.Since(started)
	if err != nil || pending != none[inputControlReply]() {
		t.Fatalf("contention leaves the write pending: %v %v", pending, err)
	}
	if elapsed >= 200*time.Millisecond {
		t.Fatalf("final validation waits neither its 600ms connection timeout nor the original request budget: %v", elapsed)
	}
	if len(h.pty.program) != 0 || inputRow(t, h, id).state != inputStateInProgress || h.hooks.count("before") != 1 {
		t.Fatal("a contended final validation writes nothing")
	}
	h.typeHuman([]byte("human"))
	h.childOutput([]byte(ordinaryScreen))
	previousSeq := h.fixture.transport.snapshot().screenSeq
	h.drain()
	if len(h.human.consumed) != 5 || h.coordinator.manualGeneration != 5 || !bytes.Equal(h.pty.manual, []byte("human")) || h.fixture.transport.snapshot().screenSeq <= previousSeq {
		t.Fatal("the fresh drain forwards the manual input and observes new output")
	}
	if h.advance() != none[inputControlReply]() || len(h.pty.program) != 0 {
		t.Fatal("still contended")
	}
	mustExecBatch(t, blocker, "ROLLBACK")
	if h.advance() != none[inputControlReply]() {
		t.Fatal("contention consumes the preceding drain")
	}
	var result submissionResult
	for {
		if !h.clock.now().Before(h.until) {
			t.Fatal("the released write completes in time")
		}
		h.drain()
		if reply := h.advance(); reply != none[inputControlReply]() {
			result = submitted(t, reply)
			break
		}
		h.clock.advance(5 * time.Millisecond)
	}
	if result.Status != submissionStatusAccepted || result.InputID != some(id) || !slices.EqualFunc(h.pty.stageBytes(), offers(ordinaryLiteral, "~"), bytes.Equal) ||
		!slices.EqualFunc(h.pty.program, [][]byte{encodedLiteral()}, bytes.Equal) || h.hooks.count("before") < 1 {
		t.Fatalf("result %+v stages %q", result, h.pty.stageBytes())
	}
}

func TestRuntimeInput_OrdinaryPrompt_Sql_PromptInheritedExpiryRefusesBeforeSqlAndAfterUncontendedSqlOnTheActualConnection(t *testing.T) {
	t.Parallel()
	for _, during := range []bool{false, true} {
		h := newInputHost(t)
		id := h.arm("prompt:sql-expiry")
		h.drain()
		setBusyTimeout(t, h.conn(), 123*time.Millisecond)
		outer := h.clock.now().Add(50 * time.Millisecond)
		state := &sqlObservation{clock: h.clock}
		if during {
			state.delay = some(outer)
		}
		observeSQL(t, h.conn(), state)
		_, err := withDatabaseBudget(context.Background(), h.conn(), deadlineUntil(outer), func(ctx context.Context, conn *dbConn) (bool, error) {
			if !during {
				passUntil(h.clock, outer)
			}
			guard, acquired, err := h.coordinator.acquireWriteGuard(ctx, conn, h.fixture.transport, h.fixture.controller, h.until)
			if guard != nil {
				guard.close()
			}
			return acquired, err
		})
		if kind, _ := cafleetKind(err); kind != errorDeadlineExceeded {
			t.Fatalf("during=%v: %v", during, err)
		}
		if state.entered.Load() != during {
			t.Fatalf("during=%v: the actual member-run SQL entry is control-separated", during)
		}
		if len(h.pty.offers) != 0 || inputRow(t, h, id).state != inputStateInProgress {
			t.Fatalf("during=%v: no write", during)
		}
	}
}

func TestRuntimeInput_OrdinaryPrompt_Sql_PromptGuardKeepsTheEarliestInheritedExpirationAfterSqlScopeRestoration(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	id := h.arm("prompt:guard-sql-bound")
	h.drain()
	before := inputRow(t, h, id)
	setBusyTimeout(t, h.conn(), 321*time.Millisecond)
	outer := h.clock.now().Add(80 * time.Millisecond)
	guard, err := withDatabaseBudget(context.Background(), h.conn(), deadlineUntil(outer), func(ctx context.Context, conn *dbConn) (*inputGuard, error) {
		guard, acquired, err := h.coordinator.acquireWriteGuard(ctx, conn, h.fixture.transport, h.fixture.controller, h.until)
		if err == nil && !acquired {
			err = runtimeError("a healthy Armed operation yields a real guard")
		}
		return guard, err
	})
	if err != nil {
		t.Fatal(err)
	}
	if busyTimeoutMillis(t, h.conn()) != 321 {
		t.Fatal("the budget restores the connection timeout")
	}
	passUntil(h.clock, outer)
	if !h.clock.now().Before(h.until) {
		t.Fatal("the request budget remains")
	}
	_, err = h.fixture.transport.writeProgrammatic(h.pty, encodedLiteral(), guard, h.until)
	if kind, _ := cafleetKind(err); kind != errorDeadlineExceeded || len(h.pty.offers) != 0 {
		t.Fatalf("the guard keeps the earliest inherited expiration: %v %q", err, h.pty.offers)
	}
	guard.close()
	if point := h.retainedCheckpoint(attemptKeyReceipt{id: id}); point.stage != submissionStageZeroBytes || point.writtenBytes != 0 {
		t.Fatalf("point %+v", point)
	}
	if inputRow(t, h, id) != before || !h.conn().raw.GetAutocommit() || busyTimeoutMillis(t, h.conn()) != 321 {
		t.Fatal("the guard leaves the row, the transaction state and the timeout unchanged")
	}
}

func TestRuntimeInput_ReservationGuards_AReceiptReservedByAnotherWriterBeforeTheReservationRepliesItAndFreesTheCoordinator(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	if h.mustHandle(h.request("prompt:raced-reservation")) != none[inputControlReply]() {
		t.Fatal("the submission waits for its reservation")
	}
	composer := h.coordinator.pending.composer()
	selection := reservationSelection{method: composer.method, reason: composer.reason}
	raced, err := reserveInputWithProvenance(context.Background(), fileConn(t, h.fixture.url), h.coordinator.run.owner(), h.coordinator.pending.request.newInput(), newReservation{selection: some(selection), deferReason: none[string]()})
	if err != nil || !raced.inserted {
		t.Fatalf("the other writer reserves the receipt first: %v", err)
	}
	var reply submissionResult
	for range 8 {
		h.drain()
		if next := h.advance(); next != none[inputControlReply]() {
			reply = submitted(t, next)
			break
		}
	}
	if reply.InputID != some(raced.operation.inputID) || h.hooks.count("reserved") != 0 {
		t.Fatalf("the durable receipt answers the submission: %+v", reply)
	}
	if h.coordinator.pending != nil || len(h.pty.offers) != 0 {
		t.Fatal("the coordinator keeps no attempt for a receipt it did not reserve")
	}
	queuedReply(t, h, h.request("prompt:after-raced-reservation"), "earlier_input")
}

func TestRuntimeInput_ReservationGuards_ReusingAnOperationKeyWithAnotherPayloadIsAConflict(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	if first := settle(t, h, h.request("prompt:reused-key")); first.Status != submissionStatusAccepted {
		t.Fatalf("first %+v", first)
	}
	written := slices.Clone(h.pty.offers)
	for _, request := range []inputControlRequest{
		literalRequest(h, "prompt:reused-key", "another literal"),
		asKind(h.request("prompt:reused-key"), inputKindExec),
	} {
		_, failure := h.handle(request)
		if failure == nil || !strings.Contains(failure.source.message(), "conflicting input operation key reuse") {
			t.Fatalf("a changed payload under the same key conflicts: %v", failure)
		}
	}
	if inputRows(t, h) != 1 || !slices.EqualFunc(h.pty.offers, written, bytes.Equal) {
		t.Fatal("a conflict reserves and writes nothing")
	}
}

func TestRuntimeInput_ReservationGuards_AZeroByteAttemptWritesTheSelectionCurrentAtItsFirstStage(t *testing.T) {
	t.Parallel()
	h := newCatalogHost(t, configuredCatalog(t, "bracketed_paste", nil), framingProgram(ordinaryLiteral, true), metadataProfile())
	h.arm("prompt:reselected")
	enablePaste(t, h)
	reply := replyAfter(t, h)
	if reply.Status != submissionStatusAccepted || !slices.EqualFunc(h.pty.program, [][]byte{framingProgram(ordinaryLiteral, true)}, bytes.Equal) {
		t.Fatalf("the paste-framed selection made before the first byte is the one written: %+v %q", reply, h.pty.written)
	}
}

func permissionHost(t *testing.T, until time.Duration) *inputHost {
	t.Helper()
	return buildInputHost(t, permissionCatalog(t), encodedLiteral(), metadataProfile(), time.Now().Add(until))
}

func closeForm(h *inputHost) {
	h.show([]byte(ordinaryScreen))
}

func resultEvents(t *testing.T, h *inputHost, inputID int64) int64 {
	t.Helper()
	return queryInt64(t, h.conn(), "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_result' AND json_extract(payload_json,'$.input_id')=?2", h.run().runID, inputID)
}

func assertNotSubmitted(t *testing.T, h *inputHost, inputID int64, reason string) {
	t.Helper()
	if input := inputRow(t, h, inputID); input.state != inputStateSettled || input.disposition != some(inputDispositionNotSubmitted) ||
		input.deliveryMethod != none[deliveryMethod]() || input.deliveryReason != some(reason) {
		t.Fatalf("input %+v", input)
	}
}

func assertNoLiteralPersisted(t *testing.T, h *inputHost) {
	t.Helper()
	for _, table := range takeTableSnapshot(t, h.conn(), func(string) bool { return true }) {
		for _, row := range table.rows {
			for _, value := range row {
				if text, ok := value.(string); ok && strings.Contains(text, ordinaryLiteral) {
					t.Fatalf("%s: %s", table.name, text)
				}
			}
		}
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_QueueAdmissionCommitsADurableReadyReceiptAndKeepsTheLiteralInMemory(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 5*time.Second)
	stableForm(t, h)
	id := queuedReply(t, h, h.request("prompt:queued"), "interaction_open")
	if input := inputRow(t, h, id); input.state != inputStateReady || input.deferredAt == none[string]() || input.deferReason != some("interaction_open") || input.deliveryMethod != none[deliveryMethod]() {
		t.Fatalf("input %+v", input)
	}
	if len(h.pty.offers) != 0 {
		t.Fatal("queue admission writes nothing")
	}
	assertNoLiteralPersisted(t, h)
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_QueueRepeatedKeyReturnsItsQueuedResultAndAConflictingDigestFails(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 5*time.Second)
	stableForm(t, h)
	id := queuedReply(t, h, h.request("prompt:repeat"), "interaction_open")
	if again := queuedReply(t, h, h.request("prompt:repeat"), "interaction_open"); again != id {
		t.Fatalf("the repeated key returns its queued result: %d", again)
	}
	if _, failure := h.handle(literalRequest(h, "prompt:repeat", "different literal")); failure == nil {
		t.Fatal("a conflicting digest fails")
	}
	if inputRows(t, h) != 1 {
		t.Fatal("one receipt")
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_QueueDrainsOnceAfterTheInteractionClosesWithoutAnotherRequest(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 8*time.Second)
	stableForm(t, h)
	id := queuedReply(t, h, h.request("prompt:after-close"), "interaction_open")
	closeForm(h)
	turnsUntil(t, h, func() bool { return delivered(t, h, id) && resultEvents(t, h, id) == 1 })
	if !slices.EqualFunc(h.pty.program, [][]byte{encodedLiteral()}, bytes.Equal) {
		t.Fatalf("program %q", h.pty.program)
	}
	if input := inputRow(t, h, id); input.deliveryMethod != some(deliveryMethodProfile) || input.deferReason != some("interaction_open") {
		t.Fatalf("input %+v", input)
	}
	for range 20 {
		h.drain()
		h.advance()
	}
	if resultEvents(t, h, id) != 1 || len(h.pty.program) != 1 {
		t.Fatal("the result is published once, with no replay")
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_QueueDrainsAfterACandidateClears(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 8*time.Second)
	h.show(inspectionFixtureForm("write candidate.txt", "synthetic-only"))
	if eligibility := h.fixture.controller.health(h.fixture.writer).Eligibility; eligibility != (inputEligibilityHold{reason: gateReasonCandidate}) {
		t.Fatalf("eligibility %v", eligibility)
	}
	id := queuedReply(t, h, h.request("prompt:candidate"), "interaction_candidate")
	closeForm(h)
	turnsUntil(t, h, func() bool { return delivered(t, h, id) })
	if !slices.EqualFunc(h.pty.program, [][]byte{encodedLiteral()}, bytes.Equal) {
		t.Fatalf("program %q", h.pty.program)
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_QueueDrainsBehindAnEarlierInputWhenItCompletes(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	if h.mustHandle(h.request("prompt:first")) != none[inputControlReply]() {
		t.Fatal("the first input waits")
	}
	for len(h.pty.stages) == 0 {
		if !h.clock.now().Before(h.until) {
			t.Fatal("the first input starts")
		}
		h.drain()
		h.advance()
		h.clock.advance(5 * time.Millisecond)
	}
	second := queuedReply(t, h, h.request("prompt:second"), "earlier_input")
	turnsUntil(t, h, func() bool { return delivered(t, h, second) })
	if !slices.EqualFunc(h.pty.program, [][]byte{encodedLiteral(), encodedLiteral()}, bytes.Equal) ||
		!slices.EqualFunc(h.pty.stageBytes(), offers(ordinaryLiteral, "~", ordinaryLiteral, "~"), bytes.Equal) {
		t.Fatalf("the queued input never interleaves with the active one: %q", h.pty.stageBytes())
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_QueueTimerRetriesABusyPaneLockWithoutChildOutputAndDeliversOnceFree(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 10*time.Second)
	stableForm(t, h)
	id := queuedReply(t, h, h.request("prompt:busy"), "interaction_open")
	held, acquired, err := paneLockTryAcquire(h.fixture.url, h.placement())
	if err != nil || !acquired {
		t.Fatalf("the pane lock is free: %v %v", acquired, err)
	}
	closeForm(h)
	busyUntil := h.clock.now().Add(300 * time.Millisecond)
	for h.clock.now().Before(busyUntil) {
		h.drain()
		h.advance()
		h.clock.advance(20 * time.Millisecond)
	}
	if len(h.pty.offers) != 0 || inputRow(t, h, id).state != inputStateReady {
		t.Fatal("a busy pane lock keeps the input queued")
	}
	held.close()
	turnsUntil(t, h, func() bool { return delivered(t, h, id) })
	if !slices.EqualFunc(h.pty.program, [][]byte{encodedLiteral()}, bytes.Equal) {
		t.Fatalf("program %q", h.pty.program)
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_QueueAttemptDeadlineWithABusyPaneLockSettlesNotSubmittedAndPublishesOnce(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 12*time.Second)
	stableForm(t, h)
	id := queuedReply(t, h, h.request("prompt:deadline"), "interaction_open")
	held, acquired, err := paneLockTryAcquire(h.fixture.url, h.placement())
	if err != nil || !acquired {
		t.Fatalf("the pane lock is free: %v %v", acquired, err)
	}
	t.Cleanup(held.close)
	closeForm(h)
	started := h.clock.now()
	turnsUntil(t, h, func() bool { return inputRow(t, h, id).state == inputStateSettled })
	if spent := h.clock.now().Sub(started); spent < 5*time.Second {
		t.Fatalf("the attempt keeps its five-second budget: %v", spent)
	}
	assertNotSubmitted(t, h, id, "pane_input_busy")
	if inputRow(t, h, id).deferReason != some("interaction_open") || len(h.pty.offers) != 0 {
		t.Fatal("the settled input keeps its defer reason and wrote nothing")
	}
	turnsUntil(t, h, func() bool { return resultEvents(t, h, id) == 1 })
}

func queuedIDs(h *inputHost) []int64 {
	ids := []int64{}
	for _, queued := range h.coordinator.queue {
		ids = append(ids, queued.inputID)
	}
	return ids
}

func cancelQueued(t *testing.T, h *inputHost, inputID int64) {
	t.Helper()
	reply, failure := h.receipt(receiptReconcile{InputID: inputID, Expected: inputStateReady, Disposition: inputDispositionNotSubmitted, EvidenceRef: "audit:cancelled-before-resend"})
	if failure != nil || reply != inputControlReply(receiptSettled{InputID: inputID}) {
		t.Fatalf("the coordinator cancels the queued receipt: %#v %v", reply, failure)
	}
}

func assertCancelledZeroByte(t *testing.T, h *inputHost, inputID int64) {
	t.Helper()
	if input := inputRow(t, h, inputID); input.state != inputStateSettled || input.disposition != some(inputDispositionNotSubmitted) || input.deliveryMethod != none[deliveryMethod]() {
		t.Fatalf("input %+v", input)
	}
	if resultEvents(t, h, inputID) != 0 {
		t.Fatal("a cancelled receipt publishes no delivery result")
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_ReconcileCancelsAQueuedReadyPromptAndPopsItFromTheFifoInPlace(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 20*time.Second)
	stableForm(t, h)
	first := queuedReply(t, h, h.request("prompt:first"), "interaction_open")
	cancelled := queuedReply(t, h, h.request("prompt:cancelled"), "interaction_open")
	last := queuedReply(t, h, h.request("prompt:last"), "interaction_open")
	cancelQueued(t, h, cancelled)
	assertCancelledZeroByte(t, h, cancelled)
	if ids := queuedIDs(h); !slices.Equal(ids, []int64{first, last}) {
		t.Fatalf("the settled entry leaves the FIFO in the same call: %v", ids)
	}
	closeForm(h)
	turnsUntil(t, h, func() bool { return delivered(t, h, first) && delivered(t, h, last) })
	if !slices.EqualFunc(h.pty.program, [][]byte{encodedLiteral(), encodedLiteral()}, bytes.Equal) || h.coordinator.inputFenced {
		t.Fatalf("only the two remaining prompts are typed: %q", h.pty.program)
	}
	assertCancelledZeroByte(t, h, cancelled)
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_ReconcileCancelsAQueuedReadyExecAndPopsItFromTheFifo(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 20*time.Second)
	stableForm(t, h)
	exec := queuedReply(t, h, execRequest(h, "exec:cancelled"), "interaction_open")
	after := queuedReply(t, h, h.request("prompt:after-exec"), "interaction_open")
	cancelQueued(t, h, exec)
	assertCancelledZeroByte(t, h, exec)
	if ids := queuedIDs(h); !slices.Equal(ids, []int64{after}) {
		t.Fatalf("queue %v", ids)
	}
	closeForm(h)
	turnsUntil(t, h, func() bool { return delivered(t, h, after) })
	if !slices.EqualFunc(h.pty.program, [][]byte{encodedLiteral()}, bytes.Equal) || h.coordinator.inputFenced {
		t.Fatalf("program %q", h.pty.program)
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_HousekeepingFailsLoudlyAndFencesInputWhenAQueuedReceiptChangedOutsideTheRunnerQueue(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 20*time.Second)
	stableForm(t, h)
	id := queuedReply(t, h, h.request("prompt:changed"), "interaction_open")
	if _, err := settleZeroByteDeferred(context.Background(), h.conn(), h.run().owner(), id, inputStateReady, "target_inactive"); err != nil {
		t.Fatal(err)
	}
	err := h.coordinator.queueHousekeeping(context.Background(), h.conn(), h.fixture.controller)
	expected := fmt.Sprintf("queued input %d changed to settled outside the runner queue", id)
	if err == nil || !strings.Contains(err.Error(), expected) {
		t.Fatalf("a queued receipt settled outside the runner is an invariant violation naming %q: %v", expected, err)
	}
	if !h.coordinator.inputFenced {
		t.Fatal("the violation fences ordinary input permanently")
	}
}

func stalledPayloads(t *testing.T, h *inputHost, head int64) []jsontext.Value {
	t.Helper()
	var payloads []jsontext.Value
	for _, row := range tableRows(t, h.conn(), "SELECT payload_json FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_stalled' AND json_extract(payload_json,'$.input_id')=?2 ORDER BY event_id", h.run().runID, head) {
		payloads = append(payloads, jsontext.Value(row[0].(string)))
	}
	return payloads
}

// backdateHeadSince moves the queued head's wait past the three-minute stall bound.
func backdateHeadSince(t *testing.T, h *inputHost, inputID int64) time.Time {
	t.Helper()
	if len(h.coordinator.queue) == 0 || h.coordinator.queue[0].inputID != inputID {
		t.Fatal("only the head carries head_since")
	}
	since := h.clock.now().Add(-181 * time.Second)
	h.coordinator.queue[0].headSince = some(since)
	return since
}

// waitingHead is a head queued behind an interaction that has since closed, so no queue turn has
// started it and no live gate holds it.
func waitingHead(t *testing.T, h *inputHost, key string) int64 {
	t.Helper()
	stableForm(t, h)
	id := queuedReply(t, h, h.request(key), "interaction_open")
	closeObserved(t, h)
	return id
}

// closeObserved closes the interaction and drives frames until the observer no longer reports it.
func closeObserved(t *testing.T, h *inputHost) {
	t.Helper()
	closeForm(h)
	for ordinaryBlockerOf(h.fixture.controller.status()) != none[ordinaryBlocker]() {
		if !h.clock.now().Before(h.until) {
			t.Fatal("the observer reports the interaction closed")
		}
		h.clock.advance(5 * time.Millisecond)
		h.drain()
	}
}

func awaitStalled(t *testing.T, h *inputHost, head int64) jsontext.Value {
	t.Helper()
	for {
		h.drain()
		if payloads := stalledPayloads(t, h, head); len(payloads) > 0 {
			if len(payloads) != 1 {
				t.Fatalf("payloads %s", payloads)
			}
			return payloads[0]
		}
		if !h.clock.now().Before(h.until) {
			t.Fatal("the watchdog publishes the stalled head")
		}
		h.clock.advance(5 * time.Millisecond)
	}
}

func settlePublications(h *inputHost) {
	until := h.clock.now().Add(300 * time.Millisecond)
	for h.clock.now().Before(until) {
		h.drain()
		h.clock.advance(10 * time.Millisecond)
	}
}

func assertNoStall(t *testing.T, h *inputHost, head int64) {
	t.Helper()
	settlePublications(h)
	if payloads := stalledPayloads(t, h, head); len(payloads) != 0 {
		t.Fatalf("payloads %s", payloads)
	}
}

func stallWaitingSince(h *inputHost, since time.Time) string {
	return formatUTC(h.clock.wallNow().Add(-h.clock.now().Sub(since)))
}

func jsonInt(value int64) string {
	return strconv.FormatInt(value, 10)
}

// assertStalledOnce backdates the head past the stall bound, runs housekeeping and checks the
// single cause-free event that names the head, its wait start and one instruction.
func assertStalledOnce(t *testing.T, h *inputHost, head int64) {
	t.Helper()
	since := backdateHeadSince(t, h, head)
	housekeep(t, h)
	payload := awaitStalled(t, h, head)
	if string(jsonAt(t, payload, "input_id")) != jsonInt(head) || string(jsonAt(t, payload, "blocked_by_input_id")) != "null" {
		t.Fatalf("payload %s", payload)
	}
	if strings.TrimSpace(decodedString(t, jsonAt(t, payload, "director_action", "instruction"))) == "" {
		t.Fatalf("one instruction: %s", payload)
	}
	if got := decodedString(t, jsonAt(t, payload, "waiting_since")); got != stallWaitingSince(h, since) {
		t.Fatalf("waiting_since is the head's wait start: %s", payload)
	}
	housekeep(t, h)
	housekeep(t, h)
	settlePublications(h)
	if len(stalledPayloads(t, h, head)) != 1 {
		t.Fatal("the watchdog publishes once per head")
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_StallWatchdog_AQueuedPromptOrExecHeadPublishesOneStallEventAtTheStallBound(t *testing.T) {
	t.Parallel()
	for _, kind := range []inputKind{inputKindPrompt, inputKindExec} {
		h := permissionHost(t, 20*time.Second)
		key := kind.asStr() + ":stalled-head"
		request := asKind(literalRequest(h, key, ordinaryLiteral), kind)
		stableForm(t, h)
		head := queuedReply(t, h, request, "interaction_open")
		closeObserved(t, h)
		housekeep(t, h)
		assertNoStall(t, h, head)
		assertStalledOnce(t, h, head)
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_StallWatchdog_AnInteractionResetsTheHeadWaitAndAFreshWaitPublishesAfterItCloses(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 20*time.Second)
	head := waitingHead(t, h, "prompt:behind-reopened-form")
	backdateHeadSince(t, h, head)
	stableForm(t, h)
	housekeep(t, h)
	h.advance()
	if h.coordinator.queue[0].headSince != none[time.Time]() {
		t.Fatal("an interaction blocking the head excludes its time")
	}
	assertNoStall(t, h, head)
	closeObserved(t, h)
	housekeep(t, h)
	restarted, ok := h.coordinator.queue[0].headSince.get()
	if !ok || h.clock.now().Sub(restarted) >= 10*time.Second {
		t.Fatal("the head's wait restarts when the interaction closes")
	}
	assertNoStall(t, h, head)
	assertStalledOnce(t, h, head)
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_StallWatchdog_EachHeadPublishesItsOwnSingleEvent(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 20*time.Second)
	stableForm(t, h)
	first := queuedReply(t, h, h.request("prompt:first-head"), "interaction_open")
	second := queuedReply(t, h, h.request("prompt:second-head"), "interaction_open")
	closeObserved(t, h)
	assertStalledOnce(t, h, first)
	cancelQueued(t, h, first)
	housekeep(t, h)
	assertNoStall(t, h, second)
	assertStalledOnce(t, h, second)
	if len(stalledPayloads(t, h, first)) != 1 {
		t.Fatal("the first head keeps its single event")
	}
}

const reconcileInstruction = "Inspect the uncertain input and the member's pane, then settle the input with input reconcile from that evidence. Queued input and message previews resume once it settles."

// uncertainBlocker is an uncertain prompt of the host's run, reserved and stopped outside the
// queue.
func uncertainRunBlocker(t *testing.T, h *inputHost, key string) int64 {
	t.Helper()
	ctx := context.Background()
	owner := h.run().owner()
	input, err := reserveInput(ctx, h.conn(), owner, newInput{operationKey: key, kind: inputKindPrompt, payloadDigest: strings.Repeat("d", 64)})
	if err != nil {
		t.Fatal(err)
	}
	uncertain, err := recordInputUncertain(ctx, h.conn(), owner, input.inputID, stopCauseWriteFailed)
	if err != nil {
		t.Fatal(err)
	}
	return uncertain.inputID
}

func settleBlocker(t *testing.T, h *inputHost, blocker int64) {
	t.Helper()
	if _, err := settleInput(context.Background(), h.conn(), h.run().owner(), blocker, inputStateUncertain, inputDispositionSubmitted, "audit:pane-shows-the-input"); err != nil {
		t.Fatal(err)
	}
}

func queuePreviewMessage(t *testing.T, h *inputHost, body string) {
	t.Helper()
	if _, err := insertMessageValidated(context.Background(), h.conn(), h.run().directorMemberID, h.run().memberID, body); err != nil {
		t.Fatal(err)
	}
}

type blockedRow struct {
	payload           jsontext.Value
	notificationState string
	archiveReason     any
}

func blockedRows(t *testing.T, h *inputHost, blocker int64) []blockedRow {
	t.Helper()
	var rows []blockedRow
	for _, row := range tableRows(t, h.conn(), "SELECT payload_json,notification_state,archive_reason FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_stalled' AND json_extract(payload_json,'$.blocked_by_input_id')=?2 ORDER BY event_seq", h.run().runID, blocker) {
		rows = append(rows, blockedRow{payload: jsontext.Value(row[0].(string)), notificationState: row[1].(string), archiveReason: row[2]})
	}
	return rows
}

func backdateBlockedSince(t *testing.T, h *inputHost, blocker int64) time.Time {
	t.Helper()
	wait := h.coordinator.blockedWait
	if wait == nil || wait.blockerInputID != blocker {
		t.Fatalf("a blocked wait on %d is recorded: %+v", blocker, wait)
	}
	wait.since = h.clock.now().Add(-181 * time.Second)
	return wait.since
}

func awaitBlocked(t *testing.T, h *inputHost, blocker int64, count int) {
	t.Helper()
	for len(blockedRows(t, h, blocker)) < count {
		if !h.clock.now().Before(h.until) {
			t.Fatal("the blocked wait publishes its record")
		}
		h.clock.advance(5 * time.Millisecond)
		h.drain()
	}
}

// assertBlockedOnce backdates the blocked wait, runs housekeeping and checks the record that names
// the blocker, its wait start and the reconcile action, published once.
func assertBlockedOnce(t *testing.T, h *inputHost, blocker int64, published int) {
	t.Helper()
	since := backdateBlockedSince(t, h, blocker)
	housekeep(t, h)
	h.drain()
	awaitBlocked(t, h, blocker, published)
	rows := blockedRows(t, h, blocker)
	payload := rows[len(rows)-1].payload
	if string(jsonAt(t, payload, "input_id")) != "null" || string(jsonAt(t, payload, "message_id")) != "null" || string(jsonAt(t, payload, "blocked_by_input_id")) != jsonInt(blocker) {
		t.Fatalf("payload %s", payload)
	}
	if decodedString(t, jsonAt(t, payload, "waiting_since")) != stallWaitingSince(h, since) ||
		decodedString(t, jsonAt(t, payload, "director_action", "kind")) != "reconcile_uncertain_input" ||
		decodedString(t, jsonAt(t, payload, "director_action", "instruction")) != reconcileInstruction {
		t.Fatalf("payload %s", payload)
	}
	argv := decodedArray(t, jsonAt(t, payload, "director_action", "inspect_argv"))
	var head []string
	for _, entry := range argv[:5] {
		head = append(head, decodedString(t, entry))
	}
	if !slices.Equal(head, []string{"cafleet", "input", "show", jsonInt(blocker), "--json"}) {
		t.Fatalf("the reconcile action names its inspect command: %s", payload)
	}
	if !h.coordinator.blockedWait.stalled {
		t.Fatal("the wait is marked stalled")
	}
	housekeep(t, h)
	housekeep(t, h)
	settlePublications(h)
	if len(blockedRows(t, h, blocker)) != published {
		t.Fatal("the blocked wait publishes once per wait")
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_StallWatchdog_AHeadBehindAnUncertainInputPublishesOneBlockedRecordAndNoHeadStall(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 20*time.Second)
	head := waitingHead(t, h, "prompt:behind-uncertain")
	blocker := uncertainRunBlocker(t, h, "prompt:uncertain-blocker")
	housekeep(t, h)
	wait := h.coordinator.blockedWait
	if wait == nil || wait.blockerInputID != blocker || wait.stalled {
		t.Fatalf("a waiting head starts the blocked wait: %+v", wait)
	}
	if h.coordinator.queue[0].headSince != none[time.Time]() || len(blockedRows(t, h, blocker)) != 0 {
		t.Fatal("a blocker suspends the head's own watch")
	}
	assertBlockedOnce(t, h, blocker, 1)
	if rows := blockedRows(t, h, blocker); rows[0].notificationState != "inbox" || rows[0].archiveReason != nil {
		t.Fatalf("rows %+v", rows)
	}
	if len(stalledPayloads(t, h, head)) != 0 {
		t.Fatal("a blocked head publishes no review_stalled_delivery")
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_StallWatchdog_APendingPreviewBehindAnUncertainInputPublishesOneBlockedRecord(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 20*time.Second)
	blocker := uncertainRunBlocker(t, h, "prompt:uncertain-blocker")
	queuePreviewMessage(t, h, "preview behind the uncertain prompt")
	housekeep(t, h)
	if wait := h.coordinator.blockedWait; wait == nil || wait.blockerInputID != blocker {
		t.Fatalf("a pending preview starts the blocked wait: %+v", wait)
	}
	assertBlockedOnce(t, h, blocker, 1)
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_StallWatchdog_ABlockedWaitClearsWithoutAWaiterAndRestartsForTheSameBlocker(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 20*time.Second)
	blocker := uncertainRunBlocker(t, h, "prompt:uncertain-blocker")
	housekeep(t, h)
	if h.coordinator.blockedWait != nil {
		t.Fatal("a blocker without a waiter starts no wait")
	}
	head := waitingHead(t, h, "prompt:first-waiter")
	housekeep(t, h)
	assertBlockedOnce(t, h, blocker, 1)
	cancelQueued(t, h, head)
	housekeep(t, h)
	if h.coordinator.blockedWait != nil {
		t.Fatal("the wait clears once no waiter remains")
	}
	waitingHead(t, h, "prompt:second-waiter")
	housekeep(t, h)
	restarted := h.coordinator.blockedWait
	if restarted == nil || restarted.blockerInputID != blocker || restarted.stalled || h.clock.now().Sub(restarted.since) >= 10*time.Second {
		t.Fatalf("a new waiter restarts the wait for the same blocker: %+v", restarted)
	}
	assertBlockedOnce(t, h, blocker, 2)
	rows := blockedRows(t, h, blocker)
	if rows[0].notificationState != "inbox" || rows[0].archiveReason != nil {
		t.Fatalf("first %+v", rows[0])
	}
	if rows[1].notificationState != "archived" || rows[1].archiveReason != "duplicate_delivery" {
		t.Fatalf("the restarted wait's record is a silent duplicate: %+v", rows[1])
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_StallWatchdog_ASettledBlockerClearsTheWaitAndRestartsTheHeadsOwnWatch(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 20*time.Second)
	head := waitingHead(t, h, "prompt:behind-uncertain")
	blocker := uncertainRunBlocker(t, h, "prompt:uncertain-blocker")
	housekeep(t, h)
	if h.coordinator.blockedWait == nil || h.coordinator.queue[0].headSince != none[time.Time]() {
		t.Fatal("the blocker suspends the head's watch")
	}
	settleBlocker(t, h, blocker)
	housekeep(t, h)
	if h.coordinator.blockedWait != nil || h.coordinator.queue[0].headSince == none[time.Time]() {
		t.Fatal("the head's ordinary watch starts afresh")
	}
	assertNoStall(t, h, head)
	assertStalledOnce(t, h, head)
	if len(blockedRows(t, h, blocker)) != 0 {
		t.Fatal("a settled blocker publishes no blocked record")
	}
}

func contendedPermissionHost(t *testing.T, until time.Duration) *inputHost {
	t.Helper()
	profile := metadataProfile()
	profile.clockLead = 0
	return buildInputHost(t, permissionCatalog(t), encodedLiteral(), profile, time.Now().Add(until))
}

// resultCause is the cause of the input's single delivery result, and whether it has one.
func resultCause(t *testing.T, h *inputHost, inputID int64) (jsontext.Value, bool) {
	t.Helper()
	rows := tableRows(t, h.conn(), "SELECT json_extract(payload_json,'$.cause') FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_result' AND json_extract(payload_json,'$.input_id')=?2", h.run().runID, inputID)
	if len(rows) != 1 {
		t.Fatalf("one delivery result for %d: %v", inputID, rows)
	}
	text, ok := rows[0][0].(string)
	return jsontext.Value(text), ok
}

func assertCause(t *testing.T, h *inputHost, inputID int64, kind opt[string]) {
	t.Helper()
	value, present := resultCause(t, h, inputID)
	want, caused := kind.get()
	if caused != present {
		t.Fatalf("cause %s, want %v", value, kind)
	}
	if caused {
		assertJSONEqual(t, value, jsontext.Value(`{"kind":"`+want+`"}`), "the published cause")
	}
}

func untilHeadLiteral(t *testing.T, h *inputHost) {
	t.Helper()
	for len(h.pty.stages) == 0 {
		if !h.clock.now().Before(h.until) {
			t.Fatal("the head's literal stage is written")
		}
		h.drain()
		h.advance()
		h.clock.advance(5 * time.Millisecond)
	}
}

func untilStarted(t *testing.T, h *inputHost, id int64) {
	t.Helper()
	for inputRow(t, h, id).state == inputStateReady {
		if !h.clock.now().Before(h.until) {
			t.Fatal("the deferred attempt starts")
		}
		h.drain()
		h.advance()
		h.clock.advance(5 * time.Millisecond)
	}
}

func reconcileUncertain(t *testing.T, h *inputHost, inputID int64) {
	t.Helper()
	reply, failure := h.receipt(receiptReconcile{InputID: inputID, Expected: inputStateUncertain, Disposition: inputDispositionAbandonedUncertain, EvidenceRef: "audit:inspected-pane"})
	if failure != nil || reply != inputControlReply(receiptSettled{InputID: inputID}) {
		t.Fatalf("the coordinator reconciles the uncertain receipt: %#v %v", reply, failure)
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_QueueOrderlyShutdownSettlesQueuedWorkNotSubmittedWithoutBytes(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 5*time.Second)
	stableForm(t, h)
	id := queuedReply(t, h, h.request("prompt:shutdown"), "interaction_open")
	h.coordinator.finishPendingAttempt()
	settled, err := h.coordinator.fence(context.Background(), h.until)
	if err != nil || !slices.Equal(settled, []int64{id}) {
		t.Fatalf("settled %v %v", settled, err)
	}
	assertNotSubmitted(t, h, id, "runner_stopped")
	for _, inputID := range settled {
		h.fixture.controller.publishDeliveryResult(inputDeliveryResultCause(inputID, submissionStageZeroBytes.asStr(), none[stopCause]()))
	}
	h.drain()
	if resultEvents(t, h, id) != 1 || len(h.pty.offers) != 0 {
		t.Fatal("the shutdown settlement publishes its deferred result")
	}
	closeForm(h)
	for range 10 {
		h.drain()
		h.advance()
	}
	if len(h.pty.offers) != 0 || resultEvents(t, h, id) != 1 {
		t.Fatal("a stopped queue is never replayed and its result is published once")
	}
}

func movePlacement(t *testing.T, h *inputHost) {
	t.Helper()
	if _, err := h.conn().execute("UPDATE member_placements SET mux_session='foreign-session' WHERE member_id=?1", h.run().memberID); err != nil {
		t.Fatal(err)
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_QueueHeadSettlesTargetChangedForAMovedPlacementAndTargetInactiveForADeregisteredMember(t *testing.T) {
	t.Parallel()
	moved := permissionHost(t, 60*time.Second)
	stableForm(t, moved)
	id := queuedReply(t, moved, moved.request("prompt:moved"), "interaction_open")
	movePlacement(t, moved)
	closeForm(moved)
	turnsUntil(t, moved, func() bool { return delivered(t, moved, id) })
	if input := inputRow(t, moved, id); len(moved.pty.offers) != 0 || input.disposition != some(inputDispositionNotSubmitted) || input.deliveryReason != some("target_changed") {
		t.Fatalf("input %+v offers %q", input, moved.pty.offers)
	}
	deregistered := permissionHost(t, 60*time.Second)
	if _, err := deregistered.conn().execute("UPDATE members SET status='deregistered' WHERE member_id=?1", deregistered.run().memberID); err != nil {
		t.Fatal(err)
	}
	if target, err := deregistered.coordinator.deferredTarget(context.Background(), deregistered.conn()); err != nil || target != some("target_inactive") {
		t.Fatalf("target %v %v", target, err)
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_QueuedStoppedInputsPublishOnceAndResumeOnlyAfterReconciliation(t *testing.T) {
	t.Parallel()
	for _, tc := range []string{"write_failure", "interaction_after_the_settle", "interaction_during_the_settle"} {
		h := permissionHost(t, 60*time.Second)
		stableForm(t, h)
		first := queuedReply(t, h, h.request("prompt:failing"), "interaction_open")
		next := queuedReply(t, h, h.request("prompt:after"), "interaction_open")
		closeForm(h)
		untilHeadLiteral(t, h)
		switch tc {
		case "write_failure":
			h.pty.failure = some[error](fixtureIOError{kind: syscall.EPIPE, message: "injected PTY write failure"})
		case "interaction_after_the_settle":
			h.clock.advance(h.stageSettle + 100*time.Millisecond)
			stableForm(t, h)
		default:
			stableForm(t, h)
		}
		for inputRow(t, h, first).state != inputStateUncertain {
			if !h.clock.now().Before(h.until) {
				t.Fatalf("%s: the head records its stopped outcome", tc)
			}
			h.drain()
			if _, failure := h.tryAdvance(); failure != nil && (tc != "write_failure" || !strings.Contains(failure.source.message(), "injected PTY write failure")) {
				t.Fatalf("%s: %v", tc, failure.asError())
			}
			h.clock.advance(5 * time.Millisecond)
		}
		if tc == "write_failure" && inputRow(t, h, first).stopCause != some(stopCauseWriteFailed) {
			t.Fatal("a failed deferred attempt records write_failed")
		}
		h.pty.failure = none[error]()
		turnsUntil(t, h, func() bool { return resultEvents(t, h, first) > 0 })
		holdTurns(h)
		recorded, ok := inputRow(t, h, first).stopCause.get()
		if resultEvents(t, h, first) != 1 || !ok {
			t.Fatalf("%s: a stopped result records its cause once", tc)
		}
		assertCause(t, h, first, some(recorded.asStr()))
		if inputRow(t, h, next).state != inputStateReady {
			t.Fatalf("%s: the next input waits", tc)
		}
		h.show([]byte(ordinaryScreen))
		offered := len(h.pty.offers)
		holdTurns(h)
		if inputRow(t, h, next).state != inputStateReady || len(h.pty.offers) != offered {
			t.Fatalf("%s: a verified empty composer leaves the FIFO stopped", tc)
		}
		reconcileUncertain(t, h, first)
		turnsUntil(t, h, func() bool { return delivered(t, h, next) })
		if tail := h.pty.offers[len(h.pty.offers)-2:]; !slices.EqualFunc(tail, offers(ordinaryLiteral, "~"), bytes.Equal) || resultEvents(t, h, first) != 1 {
			t.Fatalf("%s: the next queued input drains: %q", tc, tail)
		}
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_QueueHeadInteractionAfterItsZeroByteAttemptExpiredRequeuesInsteadOfSettling(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 60*time.Second)
	stableForm(t, h)
	id := queuedReply(t, h, h.request("prompt:expired-then-form"), "interaction_open")
	queuedReceipt := inputRow(t, h, id)
	closeForm(h)
	untilStarted(t, h, id)
	if len(h.pty.offers) != 0 {
		t.Fatal("the attempt starts with zero bytes")
	}
	h.coordinator.pending.deadline = h.clock.now().Add(-time.Millisecond)
	stableForm(t, h)
	holdTurns(h)
	if requeued := inputRow(t, h, id); requeued.state != inputStateReady || requeued.disposition != none[inputDisposition]() || requeued.deliveryMethod != none[deliveryMethod]() ||
		requeued.deferredAt != queuedReceipt.deferredAt || requeued.deferReason != some("interaction_open") || len(h.pty.offers) != 0 {
		t.Fatalf("an open interaction requeues the expired zero-byte attempt: %+v %q", requeued, h.pty.offers)
	}
	closeForm(h)
	turnsUntil(t, h, func() bool { return delivered(t, h, id) })
	if !slices.EqualFunc(h.pty.offers, offers(ordinaryLiteral, "~"), bytes.Equal) {
		t.Fatalf("the requeued input drains once after closure: %q", h.pty.offers)
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_DeferredAttemptExpiringAfterStorageContentionSettlesStorageUnavailable(t *testing.T) {
	t.Parallel()
	h := contendedPermissionHost(t, 60*time.Second)
	stableForm(t, h)
	id := queuedReply(t, h, h.request("prompt:contended"), "interaction_open")
	closeForm(h)
	for h.coordinator.pending == nil || h.coordinator.pending.phase != inputPhaseArmed {
		if !h.clock.now().Before(h.until) {
			t.Fatal("the deferred attempt arms")
		}
		h.drain()
		h.advance()
		h.clock.advance(5 * time.Millisecond)
	}
	if len(h.pty.offers) != 0 {
		t.Fatal("arming writes nothing")
	}
	writer := fileConn(t, h.fixture.url)
	h.drain()
	mustExecBatch(t, writer, "BEGIN IMMEDIATE")
	h.advance()
	mustExecBatch(t, writer, "ROLLBACK")
	if len(h.pty.offers) != 0 || inputRow(t, h, id).state != inputStateInProgress {
		t.Fatalf("contended storage writes nothing: %q", h.pty.offers)
	}
	h.coordinator.pending.deadline = h.clock.now().Add(-time.Millisecond)
	turnsUntil(t, h, func() bool { return delivered(t, h, id) })
	if len(h.pty.offers) != 0 {
		t.Fatalf("offers %q", h.pty.offers)
	}
	assertNotSubmitted(t, h, id, "storage_unavailable")
	turnsUntil(t, h, func() bool { return resultEvents(t, h, id) > 0 })
	if resultEvents(t, h, id) != 1 {
		t.Fatal("the result is published once")
	}
}

// shutDown applies the runner's orderly shutdown order: settle and publish the fenced results.
func shutDown(t *testing.T, h *inputHost) []int64 {
	t.Helper()
	ctx := context.Background()
	h.coordinator.finishPendingAttempt()
	settled, err := h.coordinator.fence(ctx, h.until)
	if err != nil {
		t.Fatal(err)
	}
	results, err := unpublishedDeliveryResults(ctx, h.conn(), h.run().runID)
	if err != nil {
		t.Fatal(err)
	}
	for _, result := range results {
		h.fixture.controller.publishDeliveryResult(inputDeliveryResultCause(result.inputID, result.stage, result.stopCause))
	}
	h.drain()
	return settled
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_QueueOrderlyShutdownSettlesAnActiveDeferredAttemptByItsWrittenBytes(t *testing.T) {
	t.Parallel()
	for _, written := range []bool{false, true} {
		h := permissionHost(t, 60*time.Second)
		stableForm(t, h)
		id := queuedReply(t, h, h.request("prompt:active"), "interaction_open")
		closeForm(h)
		if written {
			untilHeadLiteral(t, h)
		} else {
			untilStarted(t, h, id)
			if len(h.pty.offers) != 0 {
				t.Fatal("the attempt starts with zero bytes")
			}
		}
		if inputRow(t, h, id).state != inputStateInProgress {
			t.Fatalf("written=%v: the attempt is in progress", written)
		}
		if settled := shutDown(t, h); !slices.Equal(settled, []int64{id}) {
			t.Fatalf("written=%v: settled %v", written, settled)
		}
		if written {
			if input := inputRow(t, h, id); input.state != inputStateUncertain || input.stopCause != some(stopCauseRunnerStopped) {
				t.Fatalf("fence records runner_stopped for a written deferred attempt: %+v", input)
			}
			assertCause(t, h, id, some("runner_stopped"))
		} else {
			assertNotSubmitted(t, h, id, "runner_stopped")
			assertCause(t, h, id, none[string]())
		}
		if resultEvents(t, h, id) != 1 {
			t.Fatalf("written=%v: one result", written)
		}
	}
}

// queuedForAMovedTarget queues a head whose target placement then moves, so its next turn settles
// it target_changed through the terminal zero-byte transaction.
func queuedForAMovedTarget(t *testing.T, h *inputHost) int64 {
	t.Helper()
	stableForm(t, h)
	id := queuedReply(t, h, h.request("prompt:storage"), "interaction_open")
	movePlacement(t, h)
	closeForm(h)
	return id
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_QueueHeadTerminalCommitSurvivesATemporaryStorageFailureAndPublishesOnce(t *testing.T) {
	t.Parallel()
	h := contendedPermissionHost(t, 60*time.Second)
	id := queuedForAMovedTarget(t, h)
	writer := fileConn(t, h.fixture.url)
	advanceWhileBusy := func() {
		mustExecBatch(t, writer, "BEGIN IMMEDIATE")
		h.advance()
		mustExecBatch(t, writer, "ROLLBACK")
	}
	for h.coordinator.finalization == nil {
		if !h.clock.now().Before(h.until) {
			t.Fatal("the busy terminal transaction becomes a pending finalization")
		}
		h.drain()
		advanceWhileBusy()
	}
	if pending := h.coordinator.finalization; pending.inputID != id || pending.reason != "target_changed" {
		t.Fatalf("finalization %+v", pending)
	}
	advanceWhileBusy()
	if h.coordinator.finalization == nil {
		t.Fatal("a retry while storage is still busy keeps the finalization pending")
	}
	if inputRow(t, h, id).state != inputStateReady || resultEvents(t, h, id) != 0 {
		t.Fatal("the failed terminal transaction keeps the head's receipt")
	}
	turnsUntil(t, h, func() bool { return delivered(t, h, id) })
	if input := inputRow(t, h, id); input.disposition != some(inputDispositionNotSubmitted) || input.deliveryReason != some("target_changed") || len(h.pty.offers) != 0 {
		t.Fatalf("input %+v offers %q", input, h.pty.offers)
	}
	turnsUntil(t, h, func() bool { return resultEvents(t, h, id) > 0 })
	holdTurns(h)
	if resultEvents(t, h, id) != 1 {
		t.Fatal("the result is published once")
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_QueueHeadPermanentSettleFailureSurfacesInsteadOfRetrying(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 60*time.Second)
	stableForm(t, h)
	id := queuedReply(t, h, h.request("prompt:storage"), "interaction_open")
	later := queuedReply(t, h, h.request("prompt:later"), "interaction_open")
	movePlacement(t, h)
	closeForm(h)
	setAuthorizer(t, h.conn(), func(action sqlite3.AuthorizerActionCode, name3rd, _ string) sqlite3.AuthorizerReturnCode {
		if action == sqlite3.AUTH_UPDATE && name3rd == "member_input_operations" {
			return sqlite3.AUTH_DENY
		}
		return sqlite3.AUTH_OK
	})
	var failure *inputFailure
	for failure == nil {
		if !h.clock.now().Before(h.until) {
			t.Fatal("the permanent failure surfaces")
		}
		if h.coordinator.finalization != nil {
			t.Fatal("a permanent failure never becomes a retried finalization")
		}
		h.drain()
		_, failure = h.tryAdvance()
		h.clock.advance(5 * time.Millisecond)
	}
	if !strings.Contains(failure.source.message(), "not authorized") {
		t.Fatalf("failure %v", failure.asError())
	}
	clearAuthorizer(t, h.conn())
	if !h.coordinator.inputFenced {
		t.Fatal("a permanent failure fences ordinary input for recovery")
	}
	for range 40 {
		h.drain()
		h.tryAdvance() //nolint:errcheck // the fenced coordinator's later turns only must not dispatch
	}
	if inputRow(t, h, id).state != inputStateReady || inputRow(t, h, later).state != inputStateReady || len(h.pty.offers) != 0 {
		t.Fatalf("no later queued input is dispatched after the failure: %q", h.pty.offers)
	}
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_QueueHeadAcquiringThePaneLockTooLateForItsSettlesWritesZeroBytes(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 60*time.Second)
	stableForm(t, h)
	id := queuedReply(t, h, h.request("prompt:late-lock"), "interaction_open")
	busy, acquired, err := paneLockTryAcquire(h.coordinator.databaseURL, h.coordinator.placement)
	if err != nil || !acquired {
		t.Fatalf("another writer holds the pane lock: %v %v", acquired, err)
	}
	closeForm(h)
	for len(h.coordinator.queue) == 0 || h.coordinator.queue[0].attemptDeadline == none[time.Time]() {
		if !h.clock.now().Before(h.until) {
			t.Fatal("the head starts its attempt budget")
		}
		h.drain()
		h.advance()
		h.clock.advance(5 * time.Millisecond)
	}
	h.coordinator.queue[0].attemptDeadline = some(h.clock.now().Add(300 * time.Millisecond))
	busy.close()
	turnsUntil(t, h, func() bool { return delivered(t, h, id) })
	if len(h.pty.offers) != 0 {
		t.Fatalf("a lock acquired with less than one settle left writes nothing: %q", h.pty.offers)
	}
	assertNotSubmitted(t, h, id, "delivery_budget_exhausted")
}

func TestRuntimeInput_OrdinaryPrompt_DeferredQueue_QueueHeadUnderAnObserverOutageSettlesWithinTheObserverBudget(t *testing.T) {
	t.Parallel()
	h := permissionHost(t, 90*time.Second)
	stableForm(t, h)
	id := queuedReply(t, h, h.request("prompt:outage"), "interaction_open")
	closeForm(h)
	h.drain()
	backdated := false
	for !delivered(t, h, id) {
		if !h.clock.now().Before(h.until) {
			t.Fatal("the observer_unavailable attempt budget, not the 3-minute bound, settles the head")
		}
		if err := h.fixture.controller.observerFailed(observationFailureDetector, h.clock); err != nil {
			t.Fatal(err)
		}
		h.advance()
		if !backdated && len(h.coordinator.queue) > 0 && h.coordinator.queue[0].attemptDeadline != none[time.Time]() {
			h.coordinator.queue[0].attemptDeadline = some(h.clock.now().Add(300 * time.Millisecond))
			backdated = true
		}
		h.clock.advance(20 * time.Millisecond)
	}
	if !backdated || len(h.pty.offers) != 0 {
		t.Fatalf("the outage started the head's attempt budget and nothing was written: %q", h.pty.offers)
	}
	if input := inputRow(t, h, id); input.disposition != some(inputDispositionNotSubmitted) || input.deliveryMethod != none[deliveryMethod]() || input.deliveryReason != some("observer_unavailable") {
		t.Fatalf("input %+v", input)
	}
}

// cachedReceipt is a retained receipt's cached facts.
type cachedReceipt struct {
	operation        inputOperation
	stage            submissionStage
	writtenBytes     uint64
	outputSeq        uint64
	attemptAvailable bool
	leased           bool
}

func cachedFacts(t *testing.T, h *inputHost, id int64) cachedReceipt {
	t.Helper()
	retained, ok := h.coordinator.operations[attemptKeyReceipt{id: id}]
	if !ok {
		t.Fatalf("an actual retained reservation for %d", id)
	}
	return cachedReceipt{
		operation: attemptRecordReceiptOf(retained.operation), stage: retained.stage, writtenBytes: retained.writtenBytes,
		outputSeq: retained.outputSeq, attemptAvailable: retained.attemptAvailable, leased: retained.leased,
	}
}

// prefillCompleted fills the receipt cache with count completed entries modelled on the real
// retained entry template, under input IDs no durable row uses.
func prefillCompleted(t *testing.T, h *inputHost, templateID int64, count int) {
	t.Helper()
	template, ok := h.coordinator.operations[attemptKeyReceipt{id: templateID}]
	if !ok {
		t.Fatal("a real completed template")
	}
	selection, ok := template.selection.(*retainedSelectionComposer)
	if !ok {
		t.Fatal("an ordinary prompt retains its composer selection")
	}
	for index := range count {
		inputID := 1_000_000 + int64(index)
		operation := attemptRecordReceiptOf(template.operation)
		operation.inputID = inputID
		operation.operationKey = fmt.Sprintf("prompt:prefilled:%d", index)
		h.coordinator.operations[attemptKeyReceipt{id: inputID}] = &retainedInput{
			operation: attemptRecordReceipt{operation: operation},
			bytes:     slices.Clone(template.bytes),
			selection: &retainedSelectionComposer{method: selection.method, stages: slices.Clone(selection.stages), nextStage: len(selection.stages), offset: selection.offset, closing: selection.closing},
			stage:     template.stage, writtenBytes: template.writtenBytes, outputSeq: template.outputSeq,
		}
	}
}

func completeInput(t *testing.T, h *inputHost, key string) int64 {
	t.Helper()
	accepted := settle(t, h, h.request(key))
	id, ok := accepted.InputID.get()
	if accepted.Status != submissionStatusAccepted || accepted.Stage != submissionStageSubmitted || h.coordinator.pending != nil || !ok {
		t.Fatalf("accepted %+v", accepted)
	}
	return id
}

func receiptReply(t *testing.T, h *inputHost, action receiptAction) inputControlReply {
	t.Helper()
	reply, failure := h.receipt(action)
	if failure != nil {
		t.Fatal(failure.asError())
	}
	return reply
}

func clearHooks(h *inputHost) {
	h.hooks.mu.Lock()
	defer h.hooks.mu.Unlock()
	h.hooks.points = nil
}

func hookCount(h *inputHost) int {
	h.hooks.mu.Lock()
	defer h.hooks.mu.Unlock()
	return len(h.hooks.points)
}

func TestRuntimeInput_RetirementContracts_CompletedCoordinatorSettlementAndRetirementExceedsCacheCapacityWithoutReplay(t *testing.T) {
	t.Parallel()
	h := newInputHostUntil(t, time.Now().Add(30*time.Second))
	run := h.run()
	firstRequest := h.request("prompt:capacity:0")
	const evidence = "owned completed coordinator input"
	var firstSettled opt[inputOperation]
	for index := range 3 {
		key := fmt.Sprintf("prompt:capacity:%d", index)
		id := completeInput(t, h, key)
		if index == 0 {
			retained := cachedFacts(t, h, id)
			if retained.operation.state != inputStateAccepted || retained.stage != submissionStageSubmitted || retained.writtenBytes != uint64(len(encodedLiteral())) || retained.attemptAvailable || retained.leased {
				t.Fatalf("retained %+v", retained)
			}
			prefillCompleted(t, h, id, receiptCacheCapacity-1)
			if len(h.coordinator.operations) != receiptCacheCapacity {
				t.Fatal("the cache is full")
			}
			if _, failure := h.handleUntil(h.request("prompt:capacity:overflow"), h.until); failure == nil {
				t.Fatal("a full receipt cache refuses a new input")
			}
			if found, err := findInput(context.Background(), h.conn(), run.runID, "prompt:capacity:overflow"); err != nil || found != none[inputOperation]() {
				t.Fatalf("the refused input reserves nothing: %v", err)
			}
		}
		if len(h.coordinator.operations) != receiptCacheCapacity || len(h.pty.program) != index+1 || !bytes.Equal(h.pty.program[index], encodedLiteral()) || len(h.pty.manual) != 0 {
			t.Fatalf("%d: cache %d program %q", index, len(h.coordinator.operations), h.pty.program)
		}
		if reply := receiptReply(t, h, receiptReconcile{InputID: id, Expected: inputStateAccepted, Disposition: inputDispositionSubmitted, EvidenceRef: evidence}); reply != inputControlReply(receiptSettled{InputID: id}) {
			t.Fatalf("reply %#v", reply)
		}
		var settled opt[inputOperation]
		if index == 0 {
			row := inputRow(t, h, id)
			if row.runID != run.runID || row.operationKey != key || row.kind != inputKindPrompt || row.state != inputStateSettled ||
				row.disposition != some(inputDispositionSubmitted) || row.evidenceRef != some(evidence) || cachedFacts(t, h, id).operation != row {
				t.Fatalf("settled %+v", row)
			}
			settled = some(row)
		}
		if reply := receiptReply(t, h, receiptRetire{InputID: id}); reply != inputControlReply(receiptRetired{InputID: id}) {
			t.Fatalf("reply %#v", reply)
		}
		if len(h.coordinator.operations) != receiptCacheCapacity-1 || h.coordinator.pending != nil {
			t.Fatal("retirement releases exactly its own cache slot")
		}
		if row, ok := settled.get(); ok {
			if inputRow(t, h, id) != row {
				t.Fatal("retirement keeps the settled row")
			}
			firstSettled = some(row)
		}
		clearHooks(h)
	}
	counts := tableRows(t, h.conn(), "SELECT COUNT(*),COUNT(CASE WHEN state IN ('in_progress','accepted','uncertain') THEN 1 END) FROM member_input_operations WHERE run_id=?1", run.runID)
	if counts[0][0] != int64(3) || counts[0][1] != int64(0) {
		t.Fatalf("counts %v", counts)
	}
	first, ok := firstSettled.get()
	if !ok {
		t.Fatal("the first real completed operation is retained as history")
	}
	changes := h.totalChanges()
	reply, failure := h.handleUntil(firstRequest, h.until)
	if failure != nil {
		t.Fatal(failure.asError())
	}
	replayed := submitted(t, reply)
	if replayed.Status != submissionStatusAccepted || replayed.Stage != submissionStageSubmitted || replayed.InputID != some(first.inputID) {
		t.Fatalf("the settled key returns history immediately: %+v", replayed)
	}
	if len(h.coordinator.operations) != receiptCacheCapacity-1 || h.coordinator.pending != nil || h.totalChanges() != changes || inputRow(t, h, first.inputID) != first {
		t.Fatal("history costs no cache slot and no write")
	}
	if len(h.pty.offers) != 6 || len(h.pty.program) != 3 || len(h.pty.manual) != 0 || hookCount(h) != 0 {
		t.Fatalf("three staged inputs, never replayed: %q", h.pty.offers)
	}
}

func assertPtyUntouched(t *testing.T, h *inputHost) {
	t.Helper()
	if len(h.pty.offers) != 0 || len(h.pty.stages) != 0 || len(h.pty.program) != 0 || len(h.pty.manual) != 0 {
		t.Fatalf("offers %q", h.pty.offers)
	}
}

func TestRuntimeInput_RetirementContracts_LiveReceiptQueryRejectsChangedCreationStampBeforeEffects(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	const key = "prompt:receipt-cache-created-at"
	const stamp = "2000-01-02T03:04:05.000000+00:00"
	id := h.arm(key)
	cache := cachedFacts(t, h, id)
	pending := h.pendingSnapshot()
	durable := inputRow(t, h, id)
	if durable.createdAt == stamp {
		t.Fatal("the fixture stamp differs from the real one")
	}
	durable.createdAt = stamp
	if changed, err := h.conn().execute("UPDATE member_input_operations SET created_at=?1 WHERE input_id=?2", stamp, id); err != nil || changed != 1 {
		t.Fatalf("changed %d %v", changed, err)
	}
	if inputRow(t, h, id) != durable {
		t.Fatal("the row carries the changed stamp")
	}
	changes := h.totalChanges()
	hooks := hookCount(h)
	_, failure := h.receipt(receiptQuery{OperationKey: key})
	if failure == nil {
		t.Fatal("the receipt cache binds the original creation stamp")
	}
	source := failure.source
	if source.kind != errorWithDiagnostics || source.cause == nil || source.cause.kind != errorApp || source.cause.text != "receipt cache identity changed" {
		t.Fatalf("the selected receipt identity remains diagnostic context: %v", failure.asError())
	}
	quoted, err := json.Marshal(key)
	if err != nil {
		t.Fatal(err)
	}
	if want := fmt.Sprintf("run_id=%d input_id=%d operation_key=%s", h.run().runID, id, quoted); !slices.Equal(source.diagnostics, []string{want}) {
		t.Fatalf("diagnostics %q, want %q", source.diagnostics, want)
	}
	if failure.context.runID != some(h.run().runID) || failure.context.inputID != none[int64]() || failure.context.operationKey != some(key) {
		t.Fatalf("context %+v", failure.context)
	}
	if h.pendingSnapshot() != pending || cachedFacts(t, h, id) != cache || inputRow(t, h, id) != durable || h.totalChanges() != changes || hookCount(h) != hooks {
		t.Fatal("the refused query has no effect")
	}
	assertPtyUntouched(t, h)
}

func TestRuntimeInput_RetirementContracts_LiveReceiptQueryPreservesTheRealZeroByteCheckpointAndPendingWriter(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	const key = "prompt:live-receipt-query"
	id := h.arm(key)
	point := cachedFacts(t, h, id)
	if point.stage != submissionStageZeroBytes || !point.attemptAvailable || point.leased {
		t.Fatalf("point %+v", point)
	}
	h.childOutput([]byte("\x1b[2J\x1b[HLater unclassified output"))
	h.drain()
	if h.fixture.transport.snapshot().screenSeq <= point.outputSeq {
		t.Fatal("later output advances the screen")
	}
	writer := h.pendingSnapshot()
	changes := h.totalChanges()
	hooks := hookCount(h)
	present, ok := receiptReply(t, h, receiptQuery{OperationKey: key}).(receiptPresent)
	if !ok || present.InputID != id || present.Submission.InputID != some(id) || present.Submission.Status != submissionStatusUncertain ||
		present.Submission.Stage != submissionStageZeroBytes || present.Submission.OutputSeq != some(point.outputSeq) {
		t.Fatalf("Query projects actual retained facts even while a writer is Armed: %+v", present)
	}
	if reply := receiptReply(t, h, receiptQuery{OperationKey: "prompt:absent-receipt"}); reply != inputControlReply(receiptMissing{}) {
		t.Fatalf("reply %#v", reply)
	}
	stale := h.request("unused-receipt-key")
	stale.Operation = receiptOperation{action: receiptQuery{OperationKey: key}}
	stale.Identity.OwnerEpoch++
	if _, failure := h.handle(stale); failure == nil {
		t.Fatal("a stale owner epoch is refused")
	}
	if _, failure := h.receipt(receiptReconcile{InputID: id, Expected: inputStateInProgress, Disposition: inputDispositionNotSubmitted, EvidenceRef: "fixture:active-writer-is-not-zero-proof"}); failure == nil {
		t.Fatal("reconciliation preserves the actual Armed writer and its available permit")
	}
	if h.pendingSnapshot() != writer || cachedFacts(t, h, id) != point || inputRow(t, h, id) != point.operation || h.totalChanges() != changes || hookCount(h) != hooks || len(h.pty.offers) != 0 {
		t.Fatal("Query preserves exact request, bytes, phase and original deadline")
	}
	if _, err := h.conn().execute("UPDATE member_input_operations SET payload_digest=?1 WHERE input_id=?2", strings.Repeat("f", 64), id); err != nil {
		t.Fatal(err)
	}
	if _, failure := h.receipt(receiptQuery{OperationKey: key}); failure == nil {
		t.Fatal("the cached immutable identity must match its actual durable reservation")
	}
	if _, err := h.conn().execute("UPDATE member_input_operations SET payload_digest=?1 WHERE input_id=?2", point.operation.payloadDigest, id); err != nil {
		t.Fatal(err)
	}
	if deleted, err := h.conn().execute("DELETE FROM member_input_operations WHERE input_id=?1", id); err != nil || deleted != 1 {
		t.Fatalf("deleted %d %v", deleted, err)
	}
	changes = h.totalChanges()
	if _, failure := h.receipt(receiptQuery{OperationKey: key}); failure == nil {
		t.Fatal("a retained attempt without its actual reservation is an invariant error")
	}
	if h.pendingSnapshot() != writer || cachedFacts(t, h, id) != point || h.totalChanges() != changes || len(h.pty.offers) != 0 {
		t.Fatal("the refused query has no effect")
	}
}

func TestRuntimeInput_RetirementContracts_LiveReceiptReconciliationUsesCasRollsBackAndKeepsRealStageUntilRetirement(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	const key = "prompt:live-receipt-partial"
	id := h.arm(key)
	h.pty.short = some(3)
	h.drain()
	if h.advance() != none[inputControlReply]() || h.coordinator.pending == nil {
		t.Fatal("a short write keeps the attempt pending")
	}
	if written := cachedFacts(t, h, id); written.stage != submissionStagePartial || written.writtenBytes != 3 {
		t.Fatalf("written %+v", written)
	}
	h.pty.failure = some[error](fixtureIOError{kind: syscall.EPIPE, message: "injected PTY write failure"})
	var failure *inputFailure
	for range 4 {
		h.drain()
		reply, next := h.tryAdvance()
		if reply != none[inputControlReply]() {
			t.Fatal("the pending suffix fails before submission")
		}
		if failure = next; failure != nil {
			break
		}
	}
	if failure == nil || !strings.Contains(failure.source.message(), "injected PTY write failure") {
		t.Fatalf("the subsequent PTY write fails within bounded drained turns: %v", failure)
	}
	original := inputRow(t, h, id)
	if original.state != inputStateUncertain || original.stopCause != some(stopCauseWriteFailed) {
		t.Fatalf("original %+v", original)
	}
	point := cachedFacts(t, h, id)
	if point.stage != submissionStagePartial || point.writtenBytes != 3 {
		t.Fatalf("point %+v", point)
	}
	written := slices.Clone(h.pty.offers)
	action := func(expected inputState) receiptReconcile {
		return receiptReconcile{InputID: id, Expected: expected, Disposition: inputDispositionAbandonedUncertain, EvidenceRef: "fixture:observed-partial"}
	}
	if _, stale := h.receipt(action(inputStateAccepted)); stale == nil || !strings.Contains(stale.asError().Error(), "state") {
		t.Fatalf("the actual state CAS rejects a stale expected state: %v", stale)
	}
	if inputRow(t, h, id) != original || cachedFacts(t, h, id) != point {
		t.Fatal("a rejected CAS changes nothing")
	}
	mustExecBatch(t, h.conn(), "CREATE TRIGGER receipt_settlement_failure BEFORE UPDATE OF state ON member_input_operations WHEN NEW.state='settled' BEGIN SELECT RAISE(ABORT,'owned receipt settlement failure'); END")
	_, failed := h.receipt(action(inputStateUncertain))
	mustExecBatch(t, h.conn(), "DROP TRIGGER receipt_settlement_failure")
	if failed == nil || !strings.Contains(failed.asError().Error(), "owned receipt settlement failure") {
		t.Fatalf("settlement error %v", failed)
	}
	if inputRow(t, h, id) != original || cachedFacts(t, h, id) != point {
		t.Fatal("a rolled-back settlement changes nothing")
	}
	if reply := receiptReply(t, h, action(inputStateUncertain)); reply != inputControlReply(receiptSettled{InputID: id}) {
		t.Fatalf("reply %#v", reply)
	}
	settled := inputRow(t, h, id)
	if settled.state != inputStateSettled || settled.disposition != some(inputDispositionAbandonedUncertain) || settled.evidenceRef != some("fixture:observed-partial") {
		t.Fatalf("settled %+v", settled)
	}
	changes := h.totalChanges()
	if reply := receiptReply(t, h, receiptReconcile{InputID: id, Expected: inputStateSettled, Disposition: inputDispositionAbandonedUncertain, EvidenceRef: "fixture:preserve-first-evidence"}); reply != inputControlReply(receiptSettled{InputID: id}) {
		t.Fatalf("an equal repeat is settled: %#v", reply)
	}
	if _, conflict := h.receipt(receiptReconcile{InputID: id, Expected: inputStateSettled, Disposition: inputDispositionSubmitted, EvidenceRef: "fixture:conflicting-wire-disposition"}); conflict == nil {
		t.Fatal("a conflicting disposition is refused")
	}
	if h.totalChanges() != changes || inputRow(t, h, id) != settled {
		t.Fatal("repeats keep the first evidence")
	}
	if retained := cachedFacts(t, h, id); retained.stage != point.stage || retained.writtenBytes != point.writtenBytes || retained.outputSeq != point.outputSeq {
		t.Fatalf("retained %+v", retained)
	}
	hooks := hookCount(h)
	present, ok := receiptReply(t, h, receiptQuery{OperationKey: key}).(receiptPresent)
	if !ok || present.InputID != id || present.Submission.Status != submissionStatusRejected || present.Submission.Stage != submissionStagePartial ||
		present.Submission.OutputSeq != some(point.outputSeq) || hookCount(h) != hooks {
		t.Fatalf("the settled cache retains its actual prior facts: %+v", present)
	}
	if reply := receiptReply(t, h, receiptRetire{InputID: id}); reply != inputControlReply(receiptRetired{InputID: id}) {
		t.Fatalf("reply %#v", reply)
	}
	if _, retained := h.coordinator.operations[attemptKeyReceipt{id: id}]; retained {
		t.Fatal("retirement removes the cache entry")
	}
	history, ok := receiptReply(t, h, receiptQuery{OperationKey: key}).(receiptPresent)
	if !ok || history.InputID != id || history.Submission.InputID != some(id) || history.Submission.Status != submissionStatusRejected ||
		history.Submission.Stage != submissionStageUnknown || history.Submission.OutputSeq != none[uint64]() {
		t.Fatalf("the retired cache preserves durable Present history: %+v", history)
	}
	reply, repeatFailure := h.handle(h.request(key))
	if repeatFailure != nil {
		t.Fatal(repeatFailure.asError())
	}
	if repeat := submitted(t, reply); repeat.Status != submissionStatusRejected || repeat.Stage != submissionStageZeroBytes {
		t.Fatalf("the consumed key returns its submission retry projection: %+v", repeat)
	}
	if h.coordinator.pending != nil || !slices.EqualFunc(h.pty.offers, written, bytes.Equal) || inputRow(t, h, id) != settled {
		t.Fatal("a consumed key writes nothing")
	}
	if h.hooks.count("reserved") != 1 || h.hooks.count("before") != 2 || h.hooks.count("after") != 2 {
		t.Fatalf("hooks reserved=%d before=%d after=%d", h.hooks.count("reserved"), h.hooks.count("before"), h.hooks.count("after"))
	}
}

const testExpiration uint64 = 10_000_000_000

// sampleScript is a clock whose only reading is the scripted paired monotonic samples.
type sampleScript struct {
	mu      sync.Mutex
	samples []scriptedSample
	calls   int
}

type scriptedSample struct {
	sample monotonicSample
	err    error
}

func scripted(samples ...scriptedSample) *sampleScript {
	return &sampleScript{samples: samples}
}

func sampled(sample monotonicSample) scriptedSample {
	return scriptedSample{sample: sample}
}

func unavailableSample() scriptedSample {
	return scriptedSample{err: appError("fixture clock unavailable")}
}

func (s *sampleScript) now() time.Time { panic("input expiration reads only paired monotonic samples") }
func (s *sampleScript) wallNow() time.Time {
	panic("input expiration reads only paired monotonic samples")
}
func (s *sampleScript) sleep(time.Duration) {
	panic("input expiration never sleeps")
}
func (s *sampleScript) shutdownUnit() time.Duration { return shutdownUnit }

func (s *sampleScript) monotonicSample() (monotonicSample, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.calls++
	if len(s.samples) == 0 {
		panic("the operation consumes only its specified paired clock samples")
	}
	next := s.samples[0]
	s.samples = s.samples[1:]
	return next.sample, next.err
}

func (s *sampleScript) consumed(t *testing.T, calls, remaining int) {
	t.Helper()
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.calls != calls || len(s.samples) != remaining {
		t.Fatalf("calls %d remaining %d, want %d %d", s.calls, len(s.samples), calls, remaining)
	}
}

func clockAnchor() time.Time {
	return time.Now().Add(time.Minute)
}

func atMillis(origin time.Time, millis int64) time.Time {
	return origin.Add(time.Duration(millis) * time.Millisecond)
}

func atNanos(origin time.Time, nanos int64) time.Time {
	return origin.Add(time.Duration(nanos))
}

func pairedSample(before time.Time, monotonicNs uint64, after time.Time) monotonicSample {
	return monotonicSample{before: before, monotonicNs: monotonicNs, after: after}
}

func capturedExpiration(t *testing.T, deadline time.Time, clock *sampleScript, expected uint64) inputExpiration {
	t.Helper()
	expiration, err := inputExpirationCapture(deadline, clock)
	if err != nil || expiration.monotonicNs() != expected {
		t.Fatalf("expiration %d %v, want %d", expiration.monotonicNs(), err, expected)
	}
	clock.consumed(t, 1, 0)
	return expiration
}

func fixtureExpiration(t *testing.T, origin time.Time) inputExpiration {
	t.Helper()
	return capturedExpiration(t, atMillis(origin, 1000), scripted(sampled(pairedSample(origin, 9_000_000_000, origin))), testExpiration)
}

func receivedAt(t *testing.T, expiration inputExpiration, admittedAt time.Time, clock *sampleScript, expected time.Time) {
	t.Helper()
	got, err := expiration.receive(admittedAt, clock)
	if err != nil || !got.Equal(expected) {
		t.Fatalf("received %v %v, want %v", got, err, expected)
	}
	clock.consumed(t, 1, 0)
}

func oneFailure(t *testing.T, err error, clock *sampleScript, remaining int) {
	t.Helper()
	if err == nil {
		t.Fatal("the invalid clock operation returns an error")
	}
	clock.consumed(t, 1, remaining)
}

func captureErr(deadline time.Time, clock *sampleScript) error {
	_, err := inputExpirationCapture(deadline, clock)
	return err
}

func receiveErr(expiration inputExpiration, admittedAt time.Time, clock *sampleScript) error {
	_, err := expiration.receive(admittedAt, clock)
	return err
}

func TestRuntimeInput_ClockCaptureUsesLaterInstantRemainingWithoutBracketExtension(t *testing.T) {
	t.Parallel()
	anchor := clockAnchor()
	capturedExpiration(t, atMillis(anchor, 100), scripted(sampled(pairedSample(atMillis(anchor, 10), 1_000_000_000, atMillis(anchor, 30)))), 1_070_000_000)
}

func TestRuntimeInput_ClockCaptureEqualBracketPreservesOneNanosecond(t *testing.T) {
	t.Parallel()
	anchor := clockAnchor()
	capturedExpiration(t, atNanos(atMillis(anchor, 10), 1), scripted(sampled(pairedSample(atMillis(anchor, 10), 1_000_000_000, atMillis(anchor, 10)))), 1_000_000_001)
	capturedExpiration(t, atNanos(anchor, 1), scripted(sampled(pairedSample(anchor, 0, anchor))), 1)
}

func TestRuntimeInput_ClockCaptureAcceptsExactUnsignedExpirationUpperBound(t *testing.T) {
	t.Parallel()
	anchor := clockAnchor()
	capturedExpiration(t, atNanos(anchor, 5), scripted(sampled(pairedSample(anchor, math.MaxUint64-5, anchor))), math.MaxUint64)
}

func TestRuntimeInput_ClockCaptureRejectsUnsignedExpirationOverflow(t *testing.T) {
	t.Parallel()
	anchor := clockAnchor()
	clock := scripted(sampled(pairedSample(anchor, math.MaxUint64-5, anchor)))
	oneFailure(t, captureErr(atNanos(anchor, 6), clock), clock, 0)
}

func TestRuntimeInput_ClockCaptureRejectsEqualOrExpiredDeadlineWithoutResampling(t *testing.T) {
	t.Parallel()
	for _, afterMillis := range []int64{100, 101} {
		anchor := clockAnchor()
		clock := scripted(sampled(pairedSample(atMillis(anchor, 90), 1_000_000_000, atMillis(anchor, afterMillis))), sampled(pairedSample(anchor, 1_000_000_000, anchor)))
		oneFailure(t, captureErr(atMillis(anchor, 100), clock), clock, 1)
	}
}

func TestRuntimeInput_ClockCaptureRejectsReversedInstantBracket(t *testing.T) {
	t.Parallel()
	anchor := clockAnchor()
	clock := scripted(sampled(pairedSample(atMillis(anchor, 30), 1_000_000_000, atMillis(anchor, 10))))
	oneFailure(t, captureErr(atMillis(anchor, 100), clock), clock, 0)
}

func TestRuntimeInput_ClockCaptureSourceFailureConsumesOneSampleWithoutFallback(t *testing.T) {
	t.Parallel()
	anchor := clockAnchor()
	clock := scripted(unavailableSample(), sampled(pairedSample(anchor, 1_000_000_000, anchor)))
	oneFailure(t, captureErr(atMillis(anchor, 100), clock), clock, 1)
}

func TestRuntimeInput_ClockReceiveUsesEarlierInstantForConservativeDeadline(t *testing.T) {
	t.Parallel()
	anchor := clockAnchor()
	expiration := fixtureExpiration(t, anchor)
	receivedAt(t, expiration, anchor, scripted(sampled(pairedSample(atMillis(anchor, 1000), testExpiration-2_000_000_000, atMillis(anchor, 1100)))), atMillis(anchor, 3000))
}

func TestRuntimeInput_ClockReceiveCapsExpirationAtOriginalFiveSecondAdmission(t *testing.T) {
	t.Parallel()
	anchor := clockAnchor()
	expiration := fixtureExpiration(t, anchor)
	receivedAt(t, expiration, anchor, scripted(sampled(pairedSample(atMillis(anchor, 1000), testExpiration-9_000_000_000, atMillis(anchor, 1100)))), atMillis(anchor, 5000))
}

func TestRuntimeInput_ClockReceiveRejectsEqualOrPastMonotonicExpiration(t *testing.T) {
	t.Parallel()
	for _, monotonicNs := range []uint64{testExpiration, testExpiration + 1} {
		anchor := clockAnchor()
		expiration := fixtureExpiration(t, anchor)
		clock := scripted(sampled(pairedSample(atMillis(anchor, 1000), monotonicNs, atMillis(anchor, 1100))))
		oneFailure(t, receiveErr(expiration, anchor, clock), clock, 0)
	}
}

func TestRuntimeInput_ClockReceiveRejectsBracketDelayEqualToOrPastProjectedDeadline(t *testing.T) {
	t.Parallel()
	for _, bracket := range [][2]int64{{1000, 3000}, {1000, 4000}, {6000, 6100}} {
		anchor := clockAnchor()
		expiration := fixtureExpiration(t, anchor)
		clock := scripted(sampled(pairedSample(atMillis(anchor, bracket[0]), testExpiration-2_000_000_000, atMillis(anchor, bracket[1]))))
		oneFailure(t, receiveErr(expiration, anchor, clock), clock, 0)
	}
}

func TestRuntimeInput_ClockReceiveRejectsReversedInstantBracket(t *testing.T) {
	t.Parallel()
	anchor := clockAnchor()
	expiration := fixtureExpiration(t, anchor)
	clock := scripted(sampled(pairedSample(atMillis(anchor, 2000), testExpiration-2_000_000_000, atMillis(anchor, 1000))))
	oneFailure(t, receiveErr(expiration, anchor, clock), clock, 0)
}

func TestRuntimeInput_ClockReceiveSourceFailureConsumesOneSampleWithoutFallback(t *testing.T) {
	t.Parallel()
	anchor := clockAnchor()
	expiration := fixtureExpiration(t, anchor)
	clock := scripted(unavailableSample(), sampled(pairedSample(atMillis(anchor, 1000), testExpiration-2_000_000_000, atMillis(anchor, 1100))))
	oneFailure(t, receiveErr(expiration, anchor, clock), clock, 1)
}

func TestRuntimeInput_ClockReceiveReusesImmutableExpirationWithoutPhaseExtension(t *testing.T) {
	t.Parallel()
	anchor := clockAnchor()
	expiration := fixtureExpiration(t, anchor)
	clock := scripted(
		sampled(pairedSample(atMillis(anchor, 1000), testExpiration-2_000_000_000, atMillis(anchor, 1100))),
		sampled(pairedSample(atMillis(anchor, 2000), testExpiration-1_000_000_000, atMillis(anchor, 2100))),
		sampled(pairedSample(atMillis(anchor, 3000), testExpiration, atMillis(anchor, 3100))),
	)
	for index, remaining := range []int{2, 1} {
		got, err := expiration.receive(anchor, clock)
		if err != nil || !got.Equal(atMillis(anchor, 3000)) {
			t.Fatalf("receive %d: %v %v", index, got, err)
		}
		clock.consumed(t, index+1, remaining)
		if expiration.monotonicNs() != testExpiration {
			t.Fatal("the expiration is immutable")
		}
	}
	if _, err := expiration.receive(anchor, clock); err == nil {
		t.Fatal("the expired receive fails")
	}
	clock.consumed(t, 3, 0)
	if expiration.monotonicNs() != testExpiration {
		t.Fatal("the expiration is immutable")
	}
}

func TestRuntimeInput_ClockReceivePreservesPositiveSingleNanosecond(t *testing.T) {
	t.Parallel()
	anchor := clockAnchor()
	expiration := fixtureExpiration(t, anchor)
	receivedAt(t, expiration, anchor, scripted(sampled(pairedSample(atMillis(anchor, 1000), testExpiration-1, atMillis(anchor, 1000)))), atNanos(atMillis(anchor, 1000), 1))
}

func TestRuntimeInput_DeferredGuards_ADeferredAttemptLeavingAtZeroBytesReleasesThePaneLockItHeld(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		leave func(*testing.T, *inputHost)
		state inputState
	}{
		{"an interaction requeues it", func(t *testing.T, h *inputHost) { stableForm(t, h) }, inputStateReady},
		{"its expired deadline settles it", func(_ *testing.T, h *inputHost) {
			h.coordinator.pending.deadline = h.clock.now().Add(-time.Millisecond)
		}, inputStateSettled},
	}
	for _, tc := range cases {
		h := permissionHost(t, 20*time.Second)
		placement := h.placement()
		stableForm(t, h)
		id := queuedReply(t, h, h.request("prompt:deferred-pane"), "interaction_open")
		closeForm(h)
		for h.coordinator.pending == nil || h.coordinator.pending.phase != inputPhaseArmed {
			if !h.clock.now().Before(h.until) {
				t.Fatalf("%s: the deferred attempt arms", tc.name)
			}
			h.drain()
			h.advance()
			h.clock.advance(5 * time.Millisecond)
		}
		h.fixture.transport.takeInputDrain()
		if _, failure := h.coordinator.advance(context.Background(), h.conn(), h.fixture.transport, h.pty, h.fixture.controller, h.until); failure != nil {
			t.Fatal(failure.asError())
		}
		if lock, acquired, err := paneLockTryAcquire(h.fixture.url, placement); err != nil || acquired {
			if lock != nil {
				lock.close()
			}
			t.Fatalf("%s: the armed attempt holds the pane lock without a drain proof: %v %v", tc.name, acquired, err)
		}
		tc.leave(t, h)
		for range 8 {
			h.drain()
			h.advance()
			if inputRow(t, h, id).state == tc.state {
				break
			}
		}
		if inputRow(t, h, id).state != tc.state || len(h.pty.offers) != 0 {
			t.Fatalf("%s: input %+v offers %q", tc.name, inputRow(t, h, id), h.pty.offers)
		}
		lock, acquired, err := paneLockTryAcquire(h.fixture.url, placement)
		if err != nil || !acquired {
			t.Fatalf("%s: a deferred attempt leaving at zero bytes releases its pane lock: %v %v", tc.name, acquired, err)
		}
		lock.close()
	}
}

func retireFailure(t *testing.T, h *inputHost, inputID int64, deadline time.Time) *inputFailure {
	t.Helper()
	request := h.request("unused-receipt-key")
	request.Operation = receiptOperation{action: receiptRetire{InputID: inputID}}
	reply, failure := h.handleUntil(request, deadline)
	if failure == nil {
		t.Fatalf("the retirement is refused: %v", reply)
	}
	if _, retained := h.coordinator.operations[attemptKeyReceipt{id: inputID}]; !retained {
		t.Fatal("a refused retirement keeps the cache entry")
	}
	return failure
}

func reconciled(t *testing.T, h *inputHost, key string) int64 {
	t.Helper()
	id := completeInput(t, h, key)
	if reply := receiptReply(t, h, receiptReconcile{InputID: id, Expected: inputStateAccepted, Disposition: inputDispositionSubmitted, EvidenceRef: "audit:pane-shows-the-input"}); reply != inputControlReply(receiptSettled{InputID: id}) {
		t.Fatalf("reply %#v", reply)
	}
	return id
}

func TestRuntimeInput_RetirementGuards_RetiringAnUnsettledInputIsRefused(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	id := completeInput(t, h, "prompt:unsettled-retire")
	if failure := retireFailure(t, h, id, h.until); !strings.Contains(failure.source.message(), "retirement requires this run's settled input") {
		t.Fatalf("failure %v", failure.asError())
	}
}

func TestRuntimeInput_RetirementGuards_RetiringWhileAnInputWriterIsPendingIsRefused(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	id := reconciled(t, h, "prompt:settled-retire")
	if h.mustHandle(h.request("prompt:pending-writer")) != none[inputControlReply]() || h.coordinator.pending == nil {
		t.Fatal("the next submission is pending")
	}
	if failure := retireFailure(t, h, id, h.until); !strings.Contains(failure.source.message(), "retirement requires no active input writer") {
		t.Fatalf("failure %v", failure.asError())
	}
}

func TestRuntimeInput_RetirementGuards_ARetirementWhoseDeadlinePassesDuringItsCommitKeepsTheCacheEntry(t *testing.T) {
	t.Parallel()
	h := newInputHost(t)
	id := reconciled(t, h, "prompt:late-commit-retire")
	deadline := h.clock.now().Add(50 * time.Millisecond)
	var readInput, passed atomic.Bool
	setAuthorizer(t, h.conn(), func(action sqlite3.AuthorizerActionCode, name3rd, _ string) sqlite3.AuthorizerReturnCode {
		switch {
		case action == sqlite3.AUTH_READ && name3rd == "member_input_operations":
			readInput.Store(true)
		case action == sqlite3.AUTH_TRANSACTION && name3rd == "COMMIT" && readInput.Load() && !passed.Swap(true):
			passUntil(h.clock, deadline)
		}
		return sqlite3.AUTH_OK
	})
	failure := retireFailure(t, h, id, deadline)
	clearAuthorizer(t, h.conn())
	source := failure.source
	for source.kind == errorWithDiagnostics {
		source = source.cause
	}
	if source.kind != errorDeadlineExceeded || !passed.Load() {
		t.Fatalf("the post-commit check refuses the late retirement: %v (passed at commit %v)", failure.asError(), passed.Load())
	}
}
