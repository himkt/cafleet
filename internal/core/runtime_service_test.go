package core

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"golang.org/x/sys/unix"
)

const (
	directorPaneRows = 24
	directorPaneCols = 80
)

type directorPaneEventKind uint8

const (
	directorPaneCapture directorPaneEventKind = iota + 1
	directorPaneType
	directorPaneSubmit
)

type directorPaneEvent struct {
	kind  directorPaneEventKind
	chunk string
	bytes string
}

type directorPaneShown uint8

const (
	directorPaneComposer directorPaneShown = iota + 1
	directorPanePermission
	directorPanePlain
)

type directorPaneStop uint8

const (
	directorPaneStopAtCapture directorPaneStop = iota + 1
	directorPaneStopAtFirstType
)

// directorPaneState is the Director's tmux pane behind a fake tmux binary: it answers the capture
// commands from its modelled screen, appends literal keys to its composer and empties the
// composer on the submit key.
type directorPaneState struct {
	cols              int
	shown             directorPaneShown
	typed             string
	submitClears      bool
	captureFailure    opt[string]
	humanKeyAfterType opt[string]
	// lockOnSubmit is a database the pane locks exclusively when the submit key arrives, as a
	// concurrent writer that takes the lock between submission and the stamp.
	lockOnSubmit opt[string]
	heldLock     *dbConn
	// renderDelay is how many captures after each typed chunk still show the composer without
	// it, as a native composer that renders its input late.
	renderDelay int
	unrendered  int
	stopAt      directorPaneStop
	stop        *atomic.Bool
	events      []directorPaneEvent
	calls       int
}

func (s *directorPaneState) rows() []string {
	var logical []string
	switch s.shown {
	case directorPaneComposer:
		if s.typed == "" {
			logical = []string{"Synthetic state", "State end"}
		} else {
			logical = []string{syntheticComposerLine(s.typed), "State end"}
		}
	case directorPanePermission:
		logical = []string{"Synthetic permission request", "Operation: rm -rf build", "> Allow once", "  Deny once", "Request form: synthetic-only", "Selection: "}
	case directorPanePlain:
		logical = []string{"plain output"}
	default:
		panic("invalid directorPaneShown")
	}
	var wrapped []string
	for _, line := range logical {
		characters := []rune(line)
		if len(characters) == 0 {
			wrapped = append(wrapped, "")
			continue
		}
		for chunk := range slices.Chunk(characters, s.cols) {
			wrapped = append(wrapped, string(chunk))
		}
	}
	screen := make([]string, max(directorPaneRows-len(wrapped), 0))
	return append(screen, wrapped...)
}

func (s *directorPaneState) metadata() string {
	rows := s.rows()
	last := len(rows) - 1
	cursorCol := min(len([]rune(rows[last])), s.cols)
	return fmt.Sprintf("%d %d %d %d 0\n", directorPaneRows, s.cols, last, cursorCol)
}

func (s *directorPaneState) stopIf(stage directorPaneStop) {
	if s.stop != nil && s.stopAt == stage {
		s.stop.Store(true)
	}
}

type fakeDirectorPane struct {
	t     *testing.T
	mu    sync.Mutex
	state directorPaneState
}

func newFakeDirectorPane(t *testing.T) *fakeDirectorPane {
	return &fakeDirectorPane{t: t, state: directorPaneState{cols: directorPaneCols, shown: directorPaneComposer, submitClears: true}}
}

// with runs change on the pane state under its lock.
func (p *fakeDirectorPane) with(change func(*directorPaneState)) {
	p.mu.Lock()
	defer p.mu.Unlock()
	change(&p.state)
}

func (p *fakeDirectorPane) events() []directorPaneEvent {
	p.mu.Lock()
	defer p.mu.Unlock()
	return slices.Clone(p.state.events)
}

func (p *fakeDirectorPane) typedLiterals() string {
	var typed strings.Builder
	for _, event := range p.events() {
		if event.kind == directorPaneType {
			typed.WriteString(event.chunk)
		}
	}
	return typed.String()
}

func (p *fakeDirectorPane) count(kind directorPaneEventKind) int {
	count := 0
	for _, event := range p.events() {
		if event.kind == kind {
			count++
		}
	}
	return count
}

func (p *fakeDirectorPane) submits() int  { return p.count(directorPaneSubmit) }
func (p *fakeDirectorPane) captures() int { return p.count(directorPaneCapture) }

func (p *fakeDirectorPane) calls() int {
	p.mu.Lock()
	defer p.mu.Unlock()
	return p.state.calls
}

func (p *fakeDirectorPane) typed() string {
	p.mu.Lock()
	defer p.mu.Unlock()
	return p.state.typed
}

func (p *fakeDirectorPane) binaryExists(name string) bool { return name == "tmux" }

func (p *fakeDirectorPane) run(argv []string) (string, *runError) {
	p.mu.Lock()
	defer p.mu.Unlock()
	s := &p.state
	s.calls++
	if argv[0] != "tmux" {
		p.t.Errorf("the Director pane is a tmux pane: %q", argv)
		return "", &runError{kind: runErrorFailed, detail: "not tmux"}
	}
	command := argv[3:]
	switch {
	case len(command) == 5 && command[0] == "display-message" && command[1] == "-p" && command[2] == "-t":
		if !strings.Contains(command[4], "#{pane_height}") {
			p.t.Errorf("the metadata format reads the pane height: %q", argv)
		}
		if failure, set := s.captureFailure.get(); set {
			return "", &runError{kind: runErrorFailed, detail: failure}
		}
		return s.metadata(), nil
	case len(command) == 6 && slices.Equal(command[:5], []string{"capture-pane", "-p", "-e", "-N", "-t"}):
		s.events = append(s.events, directorPaneEvent{kind: directorPaneCapture})
		s.stopIf(directorPaneStopAtCapture)
		var rows []string
		if s.unrendered > 0 {
			s.unrendered--
			typed := s.typed
			s.typed = ""
			rows = s.rows()
			s.typed = typed
		} else {
			rows = s.rows()
		}
		return strings.Join(rows, "\n") + "\n", nil
	case len(command) == 6 && command[0] == "send-keys" && command[1] == "-t" && command[3] == "-l" && command[4] == "--":
		first := !slices.ContainsFunc(s.events, func(event directorPaneEvent) bool { return event.kind == directorPaneType })
		chunk := command[5]
		s.events = append(s.events, directorPaneEvent{kind: directorPaneType, chunk: chunk})
		s.typed += chunk
		s.unrendered = s.renderDelay
		if key, set := s.humanKeyAfterType.get(); set {
			s.typed += key
			s.humanKeyAfterType = none[string]()
		}
		if first {
			s.stopIf(directorPaneStopAtFirstType)
		}
		return "", nil
	case len(command) >= 4 && command[0] == "send-keys" && command[1] == "-t" && command[3] == "-H":
		var submitted []byte
		for _, hex := range command[4:] {
			value, err := strconv.ParseUint(hex, 16, 8)
			if err != nil {
				p.t.Errorf("submit key %q is hex: %v", hex, err)
			}
			submitted = append(submitted, byte(value))
		}
		s.events = append(s.events, directorPaneEvent{kind: directorPaneSubmit, bytes: string(submitted)})
		if s.submitClears {
			s.typed = ""
		}
		if path, set := s.lockOnSubmit.get(); set {
			s.lockOnSubmit = none[string]()
			s.heldLock = exclusiveDatabaseLock(p.t, path)
		}
		return "", nil
	default:
		p.t.Errorf("unexpected tmux command for the Director pane: %q", command)
		return "", &runError{kind: runErrorFailed, detail: "unexpected command"}
	}
}

func (p *fakeDirectorPane) runFor(argv []string, _ time.Duration) (string, *runError) {
	return p.run(argv)
}

// exclusiveDatabaseLock is a second connection holding the exclusive SQLite lock, as a concurrent
// writer does.
func exclusiveDatabaseLock(t *testing.T, path string) *dbConn {
	t.Helper()
	holder, err := connect(databaseURLFor(path))
	if err != nil {
		t.Fatal(err)
	}
	if err := holder.executeBatch("BEGIN EXCLUSIVE"); err != nil {
		t.Fatal(err)
	}
	return holder
}

func releaseDatabaseLock(t *testing.T, holder *dbConn) {
	t.Helper()
	if err := holder.executeBatch("ROLLBACK"); err != nil {
		t.Fatal(err)
	}
	closeConnection(t, holder)
}

func directorLimitScreen(multiplexer string, maxScalars uint64) any {
	return jo("native_version", syntheticVersion, "composer_evidence", ja(jo(
		"operation", "prompt", "multiplexer", multiplexer, "kind", "unsubmitted_literal",
		"expected_literal", jo("repeat", "a", "count", maxScalars, "prefix", ""),
	)))
}

// directorCatalog is the synthetic codex profile with a permission request rule and, when
// multiplexers is set, Director delivery on them with a maxScalars plain limit.
func directorCatalog(t *testing.T, multiplexers []string, maxScalars uint64) *profileCatalog {
	t.Helper()
	value := composerDefinition(some("0d"), some("0d"))
	permission := jclone(jget(syntheticDefinitions(), "codex", "profile", "rules", 0))
	jarray(jget(value, "codex", "profile", "rules")).push(permission)
	jset(value, jo("rule_ids", ja(syntheticRule), "fixture_ids", ja(syntheticFixture)), "codex", "profile", "capabilities", "detection")
	if multiplexers != nil {
		jset(value, directorLimitScreen("tmux", maxScalars), "codex", "screens", directorTmuxLimit)
		jset(value, directorLimitScreen("herdr", maxScalars), "codex", "screens", directorHerdrLimit)
		listed := ja()
		for _, multiplexer := range multiplexers {
			listed.push(multiplexer)
		}
		jset(value, jo(
			"coverage", "tested",
			"multiplexers", listed,
			"rule_ids", ja(emptyComposerRule, syntheticTextRuleID),
			"plain_limit", jo("unit", "scalars", "max", maxScalars),
			"fixture_ids", ja(directorTmuxLimit, directorHerdrLimit),
		), "codex", "profile", "capabilities", "director_delivery")
	}
	return mustLoadCatalog(t, value)
}

func deliveringCatalog(t *testing.T) *profileCatalog {
	t.Helper()
	return directorCatalog(t, []string{"tmux", "herdr"}, 1024)
}

// directorRun is a running member run whose fleet's root Director runs codex in a tmux pane.
type directorRun struct {
	t           *testing.T
	fixture     *runnerFixture
	clock       *testClock
	pane        *fakeDirectorPane
	diagnostics runnerDiagnostics
	stop        *atomic.Bool
}

func newDirectorRun(t *testing.T) *directorRun {
	t.Helper()
	fixture := newRunnerFixture(t)
	if _, err := sqlExecute(fixture.conn, "UPDATE member_runs SET status='running',owner_kind='runner' WHERE run_id=?1", fixture.run.runID); err != nil {
		t.Fatal(err)
	}
	if _, err := sqlExecute(fixture.conn, "UPDATE member_placements SET coding_agent='codex' WHERE member_id=?1", fixture.run.directorMemberID); err != nil {
		t.Fatal(err)
	}
	fixture.refresh(t)
	return &directorRun{
		t:           t,
		fixture:     fixture,
		clock:       virtualClock(),
		pane:        newFakeDirectorPane(t),
		diagnostics: newRunnerDiagnostics(),
		stop:        &atomic.Bool{},
	}
}

func (r *directorRun) sweep(catalog *profileCatalog) *directorSweep {
	return newDirectorSweep(r.fixture.run, catalog, r.pane, r.clock, r.diagnostics)
}

func (r *directorRun) tick(sweep *directorSweep) {
	r.t.Helper()
	if err := sweep.tick(context.Background(), r.fixture.conn, r.stop, none[time.Time]()); err != nil {
		r.t.Fatal(err)
	}
}

// tickFor ticks once per second for seconds of virtual time.
func (r *directorRun) tickFor(sweep *directorSweep, seconds int) {
	r.t.Helper()
	for range seconds {
		r.tick(sweep)
		r.clock.advance(time.Second)
	}
}

func (r *directorRun) send(from int64, body string) messageRecord {
	r.t.Helper()
	message, err := sendMessage(context.Background(), r.fixture.conn, from, r.fixture.run.directorMemberID, body)
	if err != nil {
		r.t.Fatal(err)
	}
	return message
}

func (r *directorRun) report(body string) messageRecord {
	r.t.Helper()
	return r.send(r.fixture.run.memberID, body)
}

func (r *directorRun) notified(messageID int64) bool {
	r.t.Helper()
	return queryInt64(r.t, r.fixture.conn, "SELECT notified_at IS NOT NULL FROM messages WHERE message_id=?1", messageID) == 1
}

func (r *directorRun) directorPlacement() placement {
	r.t.Helper()
	member, err := getMember(context.Background(), r.fixture.conn, r.fixture.run.directorMemberID, r.fixture.run.fleetID)
	if err != nil {
		r.t.Fatal(err)
	}
	record, ok := member.get()
	if !ok {
		r.t.Fatal("the Director is registered")
	}
	return record.placement
}

type directorDiagnostic struct {
	message string
	count   int64
}

func (r *directorRun) directorDiagnostics() []directorDiagnostic {
	entries, _ := r.diagnostics.snapshot()
	var diagnostics []directorDiagnostic
	for _, entry := range entries {
		if entry.source == runnerDiagnosticSourceDirectorDelivery {
			diagnostics = append(diagnostics, directorDiagnostic{message: entry.message, count: entry.count})
		}
	}
	return diagnostics
}

func (r *directorRun) hasDiagnostic(message string) bool {
	return slices.Contains(r.directorDiagnostics(), directorDiagnostic{message: message, count: 1})
}

func (r *directorRun) runtimeEvents() int64 {
	r.t.Helper()
	return queryInt64(r.t, r.fixture.conn, "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1", r.fixture.run.runID)
}

func (r *directorRun) secondMember() int64 {
	r.t.Helper()
	return register(r.t, r.fixture.conn, r.fixture.run.fleetID, "second", some("%7"))
}

// setCreatedAt moves the message's creation time and returns the stored message.
func (r *directorRun) setCreatedAt(messageID int64, createdAt string) messageRecord {
	r.t.Helper()
	r.execute("UPDATE messages SET created_at=?1 WHERE message_id=?2", createdAt, messageID)
	message, err := getMessage(context.Background(), r.fixture.conn, messageID)
	if err != nil {
		r.t.Fatal(err)
	}
	return message
}

func (r *directorRun) execute(sql string, args ...any) {
	r.t.Helper()
	if _, err := sqlExecute(r.fixture.conn, sql, args...); err != nil {
		r.t.Fatal(err)
	}
}

func (r *directorRun) databaseFile() string {
	r.t.Helper()
	return r.fixture.databaseFile(r.t)
}

// assertContractDiagnostics requires every stored Director diagnostic to take one of the
// specified forms.
func (r *directorRun) assertContractDiagnostics() {
	r.t.Helper()
	for _, diagnostic := range r.directorDiagnostics() {
		if !strings.Contains(diagnostic.message, "held: ") && !strings.Contains(diagnostic.message, "uncertain: ") &&
			!strings.Contains(diagnostic.message, "unavailable: ") {
			r.t.Fatalf("diagnostic %q takes a specified form", diagnostic.message)
		}
	}
}

func TestRuntimeService_DirectorSweep_AStopAfterTheFirstTmuxLiteralChunkWritesNoFurtherChunk(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	message := run.report(strings.Repeat("c", 400))
	preview := canonicalPreviewLine(message)
	if len([]rune(preview)) <= 256 {
		t.Fatal("the literal spans two chunks")
	}
	run.pane.with(func(s *directorPaneState) { s.stopAt, s.stop = directorPaneStopAtFirstType, run.stop })
	sweep := run.sweep(deliveringCatalog(t))
	run.tick(sweep)
	var chunks []string
	for _, event := range run.pane.events() {
		if event.kind == directorPaneType {
			chunks = append(chunks, event.chunk)
		}
	}
	if want := []string{string([]rune(preview)[:256])}; !slices.Equal(chunks, want) {
		t.Fatalf("chunks %q, want %q", chunks, want)
	}
	if run.pane.submits() != 0 {
		t.Fatal("no submit key")
	}
	if run.notified(message.messageID) {
		t.Fatal("the stopped preview is not stamped")
	}
	if want := fmt.Sprintf("director delivery: message %d uncertain: runner_stopped", message.messageID); !run.hasDiagnostic(want) {
		t.Fatalf("diagnostics %v lack %q", run.directorDiagnostics(), want)
	}
}

func TestRuntimeService_DirectorSweep_StorageBusy_ABusySnapshotHoldsUnderTheDoublingBackoffAndWritesNothing(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	message := run.report("waits for storage")
	sweep := run.sweep(deliveringCatalog(t))
	lock := exclusiveDatabaseLock(t, run.databaseFile())
	for _, backoff := range []time.Duration{time.Second, 2 * time.Second} {
		now := run.clock.now()
		run.tick(sweep)
		if until, set := sweep.backoffUntil.get(); !set || !until.Equal(now.Add(backoff)) {
			t.Fatalf("a %v backoff, got %v", backoff, sweep.backoffUntil)
		}
		if len(run.pane.events()) != 0 {
			t.Fatal("no capture and no byte while storage is busy")
		}
		run.clock.advance(backoff)
	}
	releaseDatabaseLock(t, lock)
	run.tick(sweep)
	if run.pane.typedLiterals() != canonicalPreviewLine(message) || run.pane.submits() != 1 || !run.notified(message.messageID) {
		t.Fatalf("typed %q, submits %d", run.pane.typedLiterals(), run.pane.submits())
	}
	if _, set := sweep.backoffUntil.get(); set {
		t.Fatal("a confirmed submission resets the backoff")
	}
	run.assertContractDiagnostics()
}

func TestRuntimeService_DirectorSweep_StorageBusy_AHoldContinuedByBusyStorageReportsStorageUnavailableAfterThreeMinutes(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	message := run.report("held behind a draft, then by busy storage")
	run.pane.with(func(s *directorPaneState) { s.typed = "a draft" })
	sweep := run.sweep(deliveringCatalog(t))
	run.tick(sweep)
	if run.pane.submits() != 0 {
		t.Fatal("the draft holds the preview")
	}
	run.clock.advance(180 * time.Second)
	lock := exclusiveDatabaseLock(t, run.databaseFile())
	run.tick(sweep)
	releaseDatabaseLock(t, lock)
	want := []directorDiagnostic{{fmt.Sprintf("director delivery: message %d held: storage_unavailable", message.messageID), 1}}
	if got := run.directorDiagnostics(); !slices.Equal(got, want) {
		t.Fatalf("diagnostics %v, want %v", got, want)
	}
	if run.notified(message.messageID) || run.pane.typed() != "a draft" {
		t.Fatalf("nothing is typed: %q", run.pane.typed())
	}
}

func TestRuntimeService_DirectorSweep_StorageBusy_BusyStorageAtTheStampAfterSubmissionIsAnUncertainPreview(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	message := run.report("stamped while storage is busy")
	path := run.databaseFile()
	run.pane.with(func(s *directorPaneState) { s.lockOnSubmit = some(path) })
	sweep := run.sweep(deliveringCatalog(t))
	now := run.clock.now()
	run.tick(sweep)
	if run.pane.submits() != 1 {
		t.Fatal("the preview was submitted")
	}
	var held *dbConn
	run.pane.with(func(s *directorPaneState) { held, s.heldLock = s.heldLock, nil })
	if held == nil {
		t.Fatal("the pane took the database lock at the submit key")
	}
	releaseDatabaseLock(t, held)
	if run.notified(message.messageID) {
		t.Fatal("the stamp did not commit")
	}
	want := []directorDiagnostic{{fmt.Sprintf("director delivery: message %d uncertain: storage_unavailable", message.messageID), 1}}
	if got := run.directorDiagnostics(); !slices.Equal(got, want) {
		t.Fatalf("diagnostics %v, want %v", got, want)
	}
	if until, set := sweep.backoffUntil.get(); !set || !until.After(now) {
		t.Fatal("the storage failure backs off")
	}
}

type paneTimeout struct {
	command string
	timeout time.Duration
}

// hungDirectorPane is the Director pane whose first metadata read blocks until released and whose
// capture then hangs for its whole timeout, as a stuck tmux client does.
type hungDirectorPane struct {
	pane     *fakeDirectorPane
	entered  chan struct{}
	once     sync.Once
	release  chan struct{}
	mu       *sync.Mutex
	timeouts *[]paneTimeout
}

func (p *hungDirectorPane) binaryExists(name string) bool { return p.pane.binaryExists(name) }

func (p *hungDirectorPane) run(argv []string) (string, *runError) { return p.pane.run(argv) }

func (p *hungDirectorPane) runFor(argv []string, timeout time.Duration) (string, *runError) {
	command := argv[3]
	p.mu.Lock()
	*p.timeouts = append(*p.timeouts, paneTimeout{command: command, timeout: timeout})
	p.mu.Unlock()
	switch command {
	case "display-message":
		first := false
		p.once.Do(func() { first = true })
		if first {
			close(p.entered)
			<-p.release
		}
		return p.pane.run(argv)
	case "capture-pane":
		time.Sleep(timeout)
		return "", &runError{kind: runErrorTimeout}
	default:
		return p.pane.run(argv)
	}
}

// joinWithin joins thread on another goroutine so a sweep that never stops fails the test instead
// of hanging it.
func joinWithin(t *testing.T, thread *directorSweepThread, shutdown time.Time, limit time.Duration) {
	t.Helper()
	joined := make(chan struct{})
	go func() {
		thread.stopAndJoin(shutdown)
		close(joined)
	}()
	select {
	case <-joined:
	case <-time.After(limit):
		t.Fatal("the sweep thread stops and joins")
	}
}

// shutdownMidCapture is the capture timeout and join instant of a sweep handed the shutdown
// deadline shutdown builds while its first capture is in flight: the pane's first metadata read
// blocks until after the handoff, then its capture hangs for its timeout.
func shutdownMidCapture(t *testing.T, run *directorRun, c clock, shutdown func() time.Time) (time.Duration, time.Time) {
	t.Helper()
	entered := make(chan struct{})
	release := make(chan struct{})
	var mu sync.Mutex
	var timeouts []paneTimeout
	thread := directorSweepThreadStartWithRunner(databaseURLFor(run.databaseFile()), run.fixture.run, deliveringCatalog(t), c, run.diagnostics,
		func() commandRunner {
			return &hungDirectorPane{pane: newFakeDirectorPane(t), entered: entered, release: release, mu: &mu, timeouts: &timeouts}
		})
	select {
	case <-entered:
	case <-time.After(realTimeBound):
		t.Fatal("the attempt reaches its first capture")
	}
	deadline := shutdown()
	joined := make(chan time.Time, 1)
	go func() {
		thread.stopAndJoin(deadline)
		joined <- time.Now()
	}()
	time.Sleep(100 * time.Millisecond)
	close(release)
	var joinedAt time.Time
	select {
	case joinedAt = <-joined:
	case <-time.After(realTimeBound):
		t.Fatal("the sweep thread stops and joins")
	}
	mu.Lock()
	defer mu.Unlock()
	for _, recorded := range timeouts {
		if recorded.command == "capture-pane" {
			return recorded.timeout, joinedAt
		}
	}
	t.Fatal("the in-flight capture ran")
	return 0, joinedAt
}

func TestRuntimeService_DirectorSweep_SweepThread_StopAndJoinReturnsByTheShutdownDeadlineWithTheInFlightAttemptClipped(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	message := run.report("in flight at shutdown")
	shutdown := time.Now().Add(600 * time.Millisecond)
	capture, joinedAt := shutdownMidCapture(t, run, systemClock{}, func() time.Time { return shutdown })
	if joinedAt.After(shutdown.Add(250 * time.Millisecond)) {
		t.Fatalf("joined %v after the shutdown deadline", joinedAt.Sub(shutdown))
	}
	if capture > 500*time.Millisecond {
		t.Fatalf("the in-flight capture runs under the clipped deadline: %v", capture)
	}
	if run.notified(message.messageID) {
		t.Fatal("the clipped attempt is not stamped")
	}
}

func TestRuntimeService_DirectorSweep_SweepThread_TheSubprocessClipMeasuresTheShutdownDeadlineOnTheInjectedClock(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	message := run.report("in flight at a virtual shutdown")
	capture, _ := shutdownMidCapture(t, run, run.clock, func() time.Time { return run.clock.now().Add(600 * time.Millisecond) })
	if capture <= 0 || capture > 600*time.Millisecond {
		t.Fatalf("the capture timeout is the virtual time left before the shutdown deadline: %v", capture)
	}
	if run.notified(message.messageID) {
		t.Fatal("the clipped attempt is not stamped")
	}
}

func TestRuntimeService_DirectorSweep_SweepThread_AConnectFailureRetriesUnderTheDoublingBackoffAndStoresADiagnostic(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	blocker := filepath.Join(run.fixture.dir, "blocker")
	if err := os.WriteFile(blocker, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	c := testClockStartingAt(time.Now())
	thread := directorSweepThreadStart(databaseURLFor(blocker+"/cafleet.db"), run.fixture.run, deliveringCatalog(t), c, run.diagnostics)
	waited := time.Now().Add(realTimeBound)
	for c.elapsed() < 62*time.Second {
		if !time.Now().Before(waited) {
			t.Fatalf("the sweep thread keeps running: %v", c.elapsed())
		}
		runtime.Gosched()
	}
	joinWithin(t, thread, c.now().Add(time.Second), realTimeBound)
	attemptsBy := func(elapsed time.Duration) int64 {
		at, backoff, attempts := time.Duration(0), time.Second, int64(0)
		for at <= elapsed {
			attempts++
			at += backoff
			backoff = min(2*backoff, 30*time.Second)
		}
		return attempts
	}
	elapsed := c.elapsed()
	expected := []int64{attemptsBy(max(elapsed-100*time.Millisecond, 0)), attemptsBy(elapsed)}
	diagnostics := run.directorDiagnostics()
	if len(diagnostics) != 1 || !strings.HasPrefix(diagnostics[0].message, "director delivery: storage unavailable: ") {
		t.Fatalf("diagnostics %v", diagnostics)
	}
	if !slices.Contains(expected, diagnostics[0].count) {
		t.Fatalf("one connect attempt at 0, 1, 3, 7, 15, 31, 61 s and every 30 s after, %v by %v: %v", expected, elapsed, diagnostics)
	}
}

func TestRuntimeService_DirectorSweep_Snapshot_TheSnapshotSelectsTheOldestUnnotifiedPendingDirectorMessageAcrossSenders(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	second := run.secondMember()
	newer := run.report("newer report")
	older := run.send(second, "older report from another member")
	run.setCreatedAt(newer.messageID, "2026-11-23T10:00:02+00:00")
	run.setCreatedAt(older.messageID, "2026-11-23T10:00:01+00:00")
	consumed := run.report("consumed report")
	run.setCreatedAt(consumed.messageID, "2026-11-23T09:00:00+00:00")
	run.execute("UPDATE messages SET status_state='completed' WHERE message_id=?1", consumed.messageID)
	stamped := run.report("stamped report")
	run.setCreatedAt(stamped.messageID, "2026-11-23T09:00:01+00:00")
	run.execute("UPDATE messages SET notified_at='2026-11-23T09:00:02+00:00' WHERE message_id=?1", stamped.messageID)
	selected, err := directorSweepSnapshot(context.Background(), run.fixture.conn, run.fixture.run, deliveringCatalog(t))
	if err != nil {
		t.Fatal(err)
	}
	target, ok := selected.get()
	if !ok {
		t.Fatal("an eligible run selects the Director head")
	}
	if target.message.messageID != older.messageID || target.placement != run.directorPlacement() {
		t.Fatalf("target message %d, placement %+v", target.message.messageID, target.placement)
	}
}

func TestRuntimeService_DirectorSweep_Snapshot_TheSnapshotSelectsNothingWithoutAnEligibleRunDirectorOrCoverage(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name         string
		change       func(*directorRun)
		multiplexers []string
	}{
		{"the member is inactive", func(run *directorRun) {
			run.execute("UPDATE members SET status='deregistered' WHERE member_id=?1", run.fixture.run.memberID)
		}, []string{"tmux", "herdr"}},
		{"the member records another run", func(run *directorRun) { run.fixture.run.runID += 1000 }, []string{"tmux", "herdr"}},
		{"the Director has no concrete pane", func(run *directorRun) {
			run.execute("UPDATE member_placements SET mux_pane_id=NULL,mux_window_id=NULL,mux_window_name=NULL WHERE member_id=?1", run.fixture.run.directorMemberID)
		}, []string{"tmux", "herdr"}},
		{"the Director backend has no Director delivery", func(*directorRun) {}, nil},
		{"the Director multiplexer is not listed", func(*directorRun) {}, []string{"herdr"}},
	}
	for _, c := range cases {
		run := newDirectorRun(t)
		run.report("pending report")
		c.change(run)
		selected, err := directorSweepSnapshot(context.Background(), run.fixture.conn, run.fixture.run, directorCatalog(t, c.multiplexers, 1024))
		if _, found := selected.get(); err == nil && found {
			t.Fatalf("%s: no Director target", c.name)
		}
	}
}

func TestRuntimeService_DirectorSweep_AVerifiedEmptyComposerReceivesTheCanonicalPreviewThenItsMessageIsStamped(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	message := run.report("results are ready")
	sweep := run.sweep(deliveringCatalog(t))
	run.tick(sweep)
	if run.pane.typedLiterals() != canonicalPreviewLine(message) || run.pane.submits() != 1 {
		t.Fatalf("typed %q, submits %d", run.pane.typedLiterals(), run.pane.submits())
	}
	if !slices.Contains(run.pane.events(), directorPaneEvent{kind: directorPaneSubmit, bytes: "\r"}) {
		t.Fatal("the prompt plan's submit bytes")
	}
	if !run.notified(message.messageID) || len(run.directorDiagnostics()) != 0 {
		t.Fatalf("stamped %v, diagnostics %v", run.notified(message.messageID), run.directorDiagnostics())
	}
}

func TestRuntimeService_DirectorSweep_DirectorMessagesAreDeliveredStrictlyOldestFirstAcrossSenders(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	second := run.secondMember()
	newer := run.report("newer report")
	older := run.send(second, "older report")
	newer = run.setCreatedAt(newer.messageID, "2026-11-23T10:00:02+00:00")
	older = run.setCreatedAt(older.messageID, "2026-11-23T10:00:01+00:00")
	sweep := run.sweep(deliveringCatalog(t))
	run.tick(sweep)
	if !run.notified(older.messageID) || run.notified(newer.messageID) {
		t.Fatal("the older message is delivered first")
	}
	run.clock.advance(time.Second)
	run.tick(sweep)
	if !run.notified(newer.messageID) {
		t.Fatal("the newer message follows")
	}
	typed := run.pane.typedLiterals()
	if strings.Index(typed, canonicalPreviewLine(older)) >= strings.Index(typed, canonicalPreviewLine(newer)) {
		t.Fatalf("typed order %q", typed)
	}
}

func TestRuntimeService_DirectorSweep_TheSweepRunsAtMostOncePerInterval(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	run.pane.with(func(s *directorPaneState) { s.typed = "draft" })
	run.report("held report")
	sweep := run.sweep(deliveringCatalog(t))
	run.tick(sweep)
	captures := run.pane.captures()
	if captures == 0 {
		t.Fatal("the first pass captures")
	}
	run.clock.advance(500 * time.Millisecond)
	run.tick(sweep)
	if run.pane.captures() != captures {
		t.Fatal("no second pass within the interval")
	}
	run.clock.advance(500 * time.Millisecond)
	run.tick(sweep)
	if run.pane.captures() <= captures {
		t.Fatal("the next interval passes again")
	}
}

func TestRuntimeService_DirectorSweep_ABusyDirectorPaneLockEndsTheTickWithoutCaptureOrBackoff(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	message := run.report("waits for the lock")
	held, acquired, err := paneLockTryAcquire(databaseURLFor(run.databaseFile()), run.directorPlacement())
	if err != nil || !acquired {
		t.Fatalf("the Director pane lock is free: %v", err)
	}
	sweep := run.sweep(deliveringCatalog(t))
	run.tick(sweep)
	if len(run.pane.events()) != 0 {
		t.Fatal("no capture behind a busy pane lock")
	}
	if _, set := sweep.backoffUntil.get(); set {
		t.Fatal("a busy pane lock arms no backoff")
	}
	if run.notified(message.messageID) {
		t.Fatal("the message waits")
	}
	held.close()
	run.clock.advance(time.Second)
	run.tick(sweep)
	if !run.notified(message.messageID) {
		t.Fatal("the released lock delivers")
	}
}

func TestRuntimeService_DirectorSweep_AConsumedOrAlreadyStampedMessageIsNeverTyped(t *testing.T) {
	t.Parallel()
	for _, state := range []string{"consumed", "stamped"} {
		run := newDirectorRun(t)
		message := run.report("read before delivery")
		update := "UPDATE messages SET status_state='completed' WHERE message_id=?1"
		if state == "stamped" {
			update = "UPDATE messages SET notified_at='2026-11-23T10:00:00+00:00' WHERE message_id=?1"
		}
		run.execute(update, message.messageID)
		sweep := run.sweep(deliveringCatalog(t))
		run.tick(sweep)
		if len(run.pane.events()) != 0 {
			t.Fatalf("%s: the pane sees nothing", state)
		}
	}
}

func TestRuntimeService_DirectorSweep_ADraftAnInteractionOrANarrowPaneHoldsThePreviewUntilTheComposerClears(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name string
		hold func(*directorPaneState)
	}{
		{"a draft", func(s *directorPaneState) { s.typed = "draft" }},
		{"an open approval", func(s *directorPaneState) { s.shown = directorPanePermission }},
		{"an unrecognized screen", func(s *directorPaneState) { s.shown = directorPanePlain }},
		{"a narrow pane", func(s *directorPaneState) { s.cols = 10 }},
	}
	for _, c := range cases {
		run := newDirectorRun(t)
		message := run.report("held while the pane is busy")
		run.pane.with(c.hold)
		sweep := run.sweep(deliveringCatalog(t))
		run.tickFor(sweep, 5)
		if run.pane.captures() == 0 {
			t.Fatalf("%s: the sweep captures", c.name)
		}
		if run.pane.typedLiterals() != "" {
			t.Fatalf("%s: nothing is typed", c.name)
		}
		if _, set := sweep.backoffUntil.get(); set {
			t.Fatalf("%s: a hold arms no backoff", c.name)
		}
		if run.notified(message.messageID) {
			t.Fatalf("%s: the held message is not stamped", c.name)
		}
		run.pane.with(func(s *directorPaneState) { s.typed, s.shown, s.cols = "", directorPaneComposer, directorPaneCols })
		run.tick(sweep)
		if run.pane.typedLiterals() != canonicalPreviewLine(message) || !run.notified(message.messageID) {
			t.Fatalf("%s: the cleared composer receives the preview: %q", c.name, run.pane.typedLiterals())
		}
	}
}

func TestRuntimeService_DirectorSweep_AHeldHeadBlocksNewerDirectorMessages(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	run.pane.with(func(s *directorPaneState) { s.typed = "draft" })
	head := run.report("held head")
	newer := run.report("newer report")
	run.setCreatedAt(head.messageID, "2026-11-23T10:00:01+00:00")
	run.setCreatedAt(newer.messageID, "2026-11-23T10:00:02+00:00")
	sweep := run.sweep(deliveringCatalog(t))
	run.tickFor(sweep, 3)
	if run.notified(head.messageID) || run.notified(newer.messageID) || run.pane.typedLiterals() != "" {
		t.Fatal("the held head blocks every newer message")
	}
}

func TestRuntimeService_DirectorSweep_ACaptureFailureHoldsWithADoublingBackoffCappedAtThirtySeconds(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	message := run.report("waits for the pane")
	run.pane.with(func(s *directorPaneState) { s.captureFailure = some("can't find pane: %0") })
	sweep := run.sweep(deliveringCatalog(t))
	expected := time.Second
	for range 7 {
		now := run.clock.now()
		run.tick(sweep)
		if until, set := sweep.backoffUntil.get(); !set || !until.Equal(now.Add(expected)) {
			t.Fatalf("backoff %v, want %v", sweep.backoffUntil, expected)
		}
		run.clock.advance(expected - time.Millisecond)
		before := run.pane.calls()
		run.tick(sweep)
		if run.pane.calls() != before {
			t.Fatal("no pass before the backoff expires")
		}
		run.clock.advance(time.Millisecond)
		expected = min(2*expected, 30*time.Second)
	}
	if expected != 30*time.Second || run.notified(message.messageID) {
		t.Fatalf("backoff %v", expected)
	}
	unavailable := slices.ContainsFunc(run.directorDiagnostics(), func(diagnostic directorDiagnostic) bool {
		return strings.HasPrefix(diagnostic.message, "director delivery: pane ") && strings.Contains(diagnostic.message, " unavailable: ")
	})
	if !unavailable {
		t.Fatalf("diagnostics %v", run.directorDiagnostics())
	}
	run.pane.with(func(s *directorPaneState) { s.captureFailure = none[string]() })
	run.tick(sweep)
	if !run.notified(message.messageID) {
		t.Fatal("the recovered pane receives the preview")
	}
	run.clock.advance(time.Second)
	next := run.report("after the recovery")
	run.tick(sweep)
	if !run.notified(next.messageID) {
		t.Fatal("a confirmed submission resets the backoff")
	}
}

func TestRuntimeService_DirectorSweep_DirectorDeliveryCreatesNoInputReceiptAndPublishesNoRuntimeEvent(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	events := run.runtimeEvents()
	run.report("delivered silently")
	sweep := run.sweep(deliveringCatalog(t))
	run.tick(sweep)
	run.pane.with(func(s *directorPaneState) { s.typed = "draft" })
	run.report("held silently")
	run.clock.advance(time.Second)
	run.tickFor(sweep, 200)
	if run.runtimeEvents() != events {
		t.Fatal("Director delivery publishes no runtime event")
	}
	if receipts := queryInt64(t, run.fixture.conn, "SELECT COUNT(*) FROM member_input_operations"); receipts != 0 {
		t.Fatalf("Director delivery creates no input receipt, found %d", receipts)
	}
}

func TestRuntimeService_DirectorSweep_AHeldMessageRecordsOneDiagnosticWhenItsHoldReachesThreeMinutes(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	run.pane.with(func(s *directorPaneState) { s.typed = "draft" })
	message := run.report("held for minutes")
	sweep := run.sweep(deliveringCatalog(t))
	run.tickFor(sweep, 179)
	if len(run.directorDiagnostics()) != 0 {
		t.Fatal("no diagnostic before three minutes")
	}
	run.tickFor(sweep, 360)
	want := []directorDiagnostic{{fmt.Sprintf("director delivery: message %d held: composer_busy", message.messageID), 1}}
	if got := run.directorDiagnostics(); !slices.Equal(got, want) {
		t.Fatalf("one diagnostic per runner and held message: %v", got)
	}
	run.pane.with(func(s *directorPaneState) { s.typed = "" })
	run.tick(sweep)
	if !run.notified(message.messageID) {
		t.Fatal("the cleared composer receives the held preview")
	}
	run.pane.with(func(s *directorPaneState) { s.shown = directorPanePermission })
	next := run.report("held behind an approval")
	run.clock.advance(time.Second)
	run.tickFor(sweep, 181)
	if want := fmt.Sprintf("director delivery: message %d held: interaction_open", next.messageID); !run.hasDiagnostic(want) {
		t.Fatalf("diagnostics %v lack %q", run.directorDiagnostics(), want)
	}
}

func TestRuntimeService_DirectorSweep_ADirectorWithoutCoverageOrOnAnUnlistedMultiplexerGetsNoCapture(t *testing.T) {
	t.Parallel()
	for _, c := range []struct {
		name         string
		multiplexers []string
	}{{"no Director delivery", nil}, {"tmux is not listed", []string{"herdr"}}} {
		run := newDirectorRun(t)
		message := run.report("never typed")
		sweep := run.sweep(directorCatalog(t, c.multiplexers, 1024))
		run.tickFor(sweep, 3)
		if len(run.pane.events()) != 0 || run.notified(message.messageID) {
			t.Fatalf("%s: the pane sees nothing", c.name)
		}
	}
}

func TestRuntimeService_DirectorSweep_APreviewAboveTheDirectorPlainLimitTypesThePointerFormWithoutAPaste(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	message := run.report(strings.Repeat("a", 300))
	sweep := run.sweep(directorCatalog(t, []string{"tmux", "herdr"}, 120))
	run.tick(sweep)
	typed := run.pane.typedLiterals()
	if typed != pointerPreviewLine(message) {
		t.Fatalf("typed %q, want the pointer form", typed)
	}
	if strings.Contains(typed, "\x1b[200~") {
		t.Fatal("no bracketed paste")
	}
	if !run.notified(message.messageID) {
		t.Fatal("the pointer preview is stamped")
	}
}

func TestRuntimeService_DirectorSweep_APreviewAtTheDirectorPlainLimitTypesInline(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	probe := run.report("x")
	overhead := len([]rune(canonicalPreviewLine(probe))) - 1
	run.execute("UPDATE messages SET notified_at='2026-11-23T10:00:00+00:00' WHERE message_id=?1", probe.messageID)
	message := run.report(strings.Repeat("b", 200-overhead))
	if length := len([]rune(canonicalPreviewLine(message))); length != 200 {
		t.Fatalf("preview length %d", length)
	}
	sweep := run.sweep(directorCatalog(t, []string{"tmux", "herdr"}, 200))
	run.tick(sweep)
	if run.pane.typedLiterals() != canonicalPreviewLine(message) {
		t.Fatalf("typed %q inline", run.pane.typedLiterals())
	}
}

// preSubmitCaptures is the captures taken between the first typed chunk and the submit key.
func preSubmitCaptures(pane *fakeDirectorPane) int {
	count, typing := 0, false
	for _, event := range pane.events() {
		switch {
		case event.kind == directorPaneType:
			typing = true
		case event.kind == directorPaneSubmit && typing:
			return count
		case event.kind == directorPaneCapture && typing:
			count++
		}
	}
	return count
}

func TestRuntimeService_DirectorSweep_APreviewRenderingWithinThePreSubmitWindowIsSubmittedAndNotified(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	message := run.report("renders after the settle")
	run.pane.with(func(s *directorPaneState) { s.renderDelay = 4 })
	sweep := run.sweep(deliveringCatalog(t))
	run.tick(sweep)
	if run.pane.typedLiterals() != canonicalPreviewLine(message) {
		t.Fatalf("typed %q", run.pane.typedLiterals())
	}
	if got := preSubmitCaptures(run.pane); got != 5 {
		t.Fatalf("the composer is re-observed until the literal renders: %d captures", got)
	}
	if run.pane.submits() != 1 || !run.notified(message.messageID) {
		t.Fatal("the submit key follows the render")
	}
	if slices.ContainsFunc(run.directorDiagnostics(), func(d directorDiagnostic) bool { return strings.Contains(d.message, "literal_mismatch") }) {
		t.Fatalf("diagnostics %v", run.directorDiagnostics())
	}
}

func TestRuntimeService_DirectorSweep_AnAppendedKeyRecordsLiteralMismatchWhenThePreSubmitWindowCloses(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	message := run.report("races a human key past the window")
	run.pane.with(func(s *directorPaneState) { s.humanKeyAfterType = some("x") })
	sweep := run.sweep(deliveringCatalog(t))
	started := run.clock.elapsed()
	run.tick(sweep)
	if waited := run.clock.elapsed() - started; waited < directorSettle+preSubmitWindow-directorVerificationPoll {
		t.Fatalf("the mismatch waits for the window to close: %v", waited)
	}
	if preSubmitCaptures(run.pane) <= 1 {
		t.Fatal("the composer is re-observed inside the window")
	}
	if run.pane.submits() != 0 || run.notified(message.messageID) {
		t.Fatal("no submit key after the mismatch")
	}
	if want := fmt.Sprintf("director delivery: message %d uncertain: literal_mismatch", message.messageID); !run.hasDiagnostic(want) {
		t.Fatalf("diagnostics %v lack %q", run.directorDiagnostics(), want)
	}
}

func TestRuntimeService_DirectorSweep_AKeyTypedDuringTheSettleLeavesThePreviewUnnotifiedAndItIsTypedAgain(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	message := run.report("races a human key")
	run.pane.with(func(s *directorPaneState) { s.humanKeyAfterType = some("x") })
	sweep := run.sweep(deliveringCatalog(t))
	run.tick(sweep)
	if run.pane.submits() != 0 || run.notified(message.messageID) {
		t.Fatal("no submit key after the mismatch")
	}
	if want := fmt.Sprintf("director delivery: message %d uncertain: literal_mismatch", message.messageID); !run.hasDiagnostic(want) {
		t.Fatalf("diagnostics %v lack %q", run.directorDiagnostics(), want)
	}
	run.clock.advance(time.Second)
	run.tick(sweep)
	if run.pane.submits() != 0 {
		t.Fatal("the leftover literal holds the composer")
	}
	run.pane.with(func(s *directorPaneState) { s.typed = "" })
	run.clock.advance(time.Second)
	run.tick(sweep)
	if run.pane.submits() != 1 || !run.notified(message.messageID) {
		t.Fatal("the cleared composer receives the preview again")
	}
}

func TestRuntimeService_DirectorSweep_AnUnconfirmedSubmissionIsNeverStamped(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	message := run.report("submitted but not confirmed")
	run.pane.with(func(s *directorPaneState) { s.submitClears = false })
	sweep := run.sweep(deliveringCatalog(t))
	run.tick(sweep)
	if run.pane.submits() != 1 || run.notified(message.messageID) {
		t.Fatal("the unconfirmed submission is not stamped")
	}
	if want := fmt.Sprintf("director delivery: message %d uncertain: submit_unconfirmed", message.messageID); !run.hasDiagnostic(want) {
		t.Fatalf("diagnostics %v lack %q", run.directorDiagnostics(), want)
	}
}

func TestRuntimeService_DirectorSweep_AStopSignalStartsNoNewWritingStage(t *testing.T) {
	t.Parallel()
	for _, c := range []struct {
		name   string
		stopAt directorPaneStop
		typed  bool
	}{
		{"a stop at the capture", directorPaneStopAtCapture, false},
		{"a stop after the first literal chunk", directorPaneStopAtFirstType, true},
	} {
		run := newDirectorRun(t)
		message := run.report("stopped mid-delivery")
		run.pane.with(func(s *directorPaneState) { s.stopAt, s.stop = c.stopAt, run.stop })
		sweep := run.sweep(deliveringCatalog(t))
		run.tick(sweep)
		if (run.pane.typedLiterals() != "") != c.typed {
			t.Fatalf("%s: typed %q", c.name, run.pane.typedLiterals())
		}
		if run.pane.submits() != 0 || run.notified(message.messageID) {
			t.Fatalf("%s: no submit key", c.name)
		}
		if want := fmt.Sprintf("director delivery: message %d uncertain: runner_stopped", message.messageID); c.typed && !run.hasDiagnostic(want) {
			t.Fatalf("%s: diagnostics %v lack %q", c.name, run.directorDiagnostics(), want)
		}
	}
}

func TestRuntimeService_DirectorSweep_TheAttemptEndsWithinTheClippedShutdownDeadline(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	message := run.report("delivered past the shutdown deadline")
	sweep := run.sweep(deliveringCatalog(t))
	shutdown := run.clock.now().Add(200 * time.Millisecond)
	if err := sweep.tick(context.Background(), run.fixture.conn, run.stop, some(shutdown)); err != nil {
		t.Fatal(err)
	}
	if run.clock.now().After(shutdown) {
		t.Fatal("the attempt never outlives the shutdown deadline")
	}
	if run.pane.submits() != 0 || run.notified(message.messageID) {
		t.Fatal("the 500 ms settle cannot finish in time")
	}
}

func TestRuntimeService_DirectorSweep_AMessageSentWhileNoRunnerIsLiveIsDeliveredByALaterRunner(t *testing.T) {
	t.Parallel()
	run := newDirectorRun(t)
	message := run.report("sent while no runner sweeps")
	run.clock.advance(60 * time.Second)
	if run.notified(message.messageID) || len(run.pane.events()) != 0 {
		t.Fatal("nothing delivers without a live runner")
	}
	later := run.sweep(deliveringCatalog(t))
	run.tick(later)
	if run.pane.typedLiterals() != canonicalPreviewLine(message) || !run.notified(message.messageID) {
		t.Fatalf("the later runner delivers: %q", run.pane.typedLiterals())
	}
}

func clientPair(t *testing.T) (*client, int) {
	t.Helper()
	fds := cloexecSocketpair(t)
	for _, fd := range fds {
		if err := unix.SetNonblock(fd, true); err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { closeFD(t, fd) })
	}
	return &client{id: 1, fd: fds[0], decoder: newControlDecoder()}, fds[1]
}

// waitPast spins until the wall clock passes previous, so the next sample is strictly later.
func waitPast(t *testing.T, previous, deadline time.Time) {
	t.Helper()
	for !time.Now().After(previous) {
		if !time.Now().Before(deadline) {
			t.Fatal("fixture clock advances within its bound")
		}
		runtime.Gosched()
	}
}

func collectFrames(t *testing.T, c *client, count int, deadline time.Time) []admittedControlFrame {
	t.Helper()
	var frames []admittedControlFrame
	for len(frames) < count {
		if !time.Now().Before(deadline) {
			t.Fatal("bounded actual client polling")
		}
		polled, err := c.poll(systemClock{})
		if err != nil {
			t.Fatal(err)
		}
		frames = append(frames, polled...)
		runtime.Gosched()
	}
	if len(frames) != count {
		t.Fatalf("frames %d, want %d", len(frames), count)
	}
	return frames
}

// firstHeaderByte sends one byte and polls it, returning the instants that bracket the poll that
// consumed it.
func firstHeaderByte(t *testing.T, c *client, peer int, b byte, deadline time.Time) (time.Time, time.Time) {
	t.Helper()
	if err := c.decoder.finish(); err != nil {
		t.Fatal(err)
	}
	if err := writeFixtureBytes(peer, []byte{b}, deadline); err != nil {
		t.Fatal(err)
	}
	before := time.Now()
	frames, err := c.poll(systemClock{})
	if err != nil || len(frames) != 0 {
		t.Fatalf("the first header byte completes no frame: %d, %v", len(frames), err)
	}
	after := time.Now()
	if c.decoder.finish() == nil {
		t.Fatal("this actual poll consumed the first header byte")
	}
	return before, after
}

// fixedSampleClock is the system clock whose monotonic samples are one scripted reading.
type fixedSampleClock struct {
	systemClock
	sample  monotonicSample
	samples int
}

func (c *fixedSampleClock) monotonicSample() (monotonicSample, error) {
	c.samples++
	return c.sample, nil
}

func TestRuntimeService_InputAdmissionTests_ClientEmptyPollSuppliesNoAdmissionTime(t *testing.T) {
	t.Parallel()
	c, peer := clientPair(t)
	deadline := time.Now().Add(realTimeBound)
	if err := c.decoder.finish(); err != nil {
		t.Fatal(err)
	}
	emptyAt := time.Now()
	if frames, err := c.poll(systemClock{}); err != nil || len(frames) != 0 {
		t.Fatalf("an empty poll admits nothing: %d, %v", len(frames), err)
	}
	if err := c.decoder.finish(); err != nil {
		t.Fatal(err)
	}
	waitPast(t, emptyAt, deadline)
	body := []byte(`{"protocol_version":1,"kind":"health"}`)
	bytes := independentFrame(body)
	before, end := firstHeaderByte(t, c, peer, bytes[0], deadline)
	waitPast(t, end, deadline)
	if err := writeFixtureBytes(peer, bytes[1:], deadline); err != nil {
		t.Fatal(err)
	}
	frames := collectFrames(t, c, 1, deadline)
	admitted := frames[0].admittedAt
	if !admitted.After(emptyAt) || admitted.Before(before) || admitted.After(end) {
		t.Fatalf("admitted at %v, want within [%v, %v] after %v", admitted, before, end, emptyAt)
	}
	if string(frames[0].body) != string(body) {
		t.Fatalf("body %q", frames[0].body)
	}
	if err := c.decoder.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestRuntimeService_InputAdmissionTests_ClientFramesKeepFirstTimeThroughCoalescenceQueueAndConversion(t *testing.T) {
	t.Parallel()
	c, peer := clientPair(t)
	deadline := time.Now().Add(realTimeBound)
	firstBody := []byte(`{"protocol_version":1,"kind":"health"}`)
	secondBody := []byte(`{"protocol_version":1,"kind":"shutdown"}`)
	bytes := independentFrame(firstBody)
	before, firstAfter := firstHeaderByte(t, c, peer, bytes[0], deadline)
	waitPast(t, firstAfter, deadline)
	laterBefore := time.Now()
	if err := writeFixtureBytes(peer, slices.Concat(bytes[1:], independentFrame(secondBody)), deadline); err != nil {
		t.Fatal(err)
	}
	frames := collectFrames(t, c, 2, deadline)
	laterAfter := time.Now()
	if string(frames[0].body) != string(firstBody) || string(frames[1].body) != string(secondBody) {
		t.Fatalf("bodies %q %q", frames[0].body, frames[1].body)
	}
	times := []time.Time{frames[0].admittedAt, frames[1].admittedAt}
	if times[0].Before(before) || times[0].After(firstAfter) {
		t.Fatalf("the first frame keeps its first header time: %v", times[0])
	}
	if times[1].Before(laterBefore) || times[1].After(laterAfter) {
		t.Fatalf("the second frame keeps its own header time: %v", times[1])
	}
	waitPast(t, laterAfter, deadline)
	for index, expected := range times {
		actual := frames[index]
		if !actual.admittedAt.Equal(expected) {
			t.Fatalf("frame %d admitted at %v, want %v", index, actual.admittedAt, expected)
		}
		now := time.Now()
		sampled := &fixedSampleClock{sample: monotonicSample{before: now, monotonicNs: 1_000_000_000, after: now}}
		converted, err := receiveInputExpiration(11_000_000_000, actual.admittedAt, sampled)
		if err != nil {
			t.Fatal(err)
		}
		if !converted.Equal(actual.admittedAt.Add(5 * time.Second)) {
			t.Fatalf("frame %d converts to %v, want %v", index, converted, actual.admittedAt.Add(5*time.Second))
		}
		if sampled.samples != 1 {
			t.Fatalf("frame %d's conversion takes one monotonic sample, took %d", index, sampled.samples)
		}
	}
	if err := c.decoder.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestRuntimeService_InputAdmissionTests_ClientRejectsPartialEof(t *testing.T) {
	t.Parallel()
	deadline := time.Now().Add(realTimeBound)
	partial, peer := clientPair(t)
	firstHeaderByte(t, partial, peer, 0, deadline)
	if err := unix.Shutdown(peer, unix.SHUT_WR); err != nil {
		t.Fatal(err)
	}
	if _, err := partial.poll(systemClock{}); err == nil {
		t.Fatal("a partial frame at EOF is refused")
	}
}

func TestRuntimeService_PreviewSweep_TheRunWideBackoffStartsAtOneSecondDoublesToThirtyAndResets(t *testing.T) {
	t.Parallel()
	var backoff sweepBackoff
	if backoff.delay() != 0 {
		t.Fatalf("a fresh backoff is zero, got %v", backoff.delay())
	}
	var delays []time.Duration
	for range 8 {
		backoff.transient()
		delays = append(delays, backoff.delay())
	}
	want := []time.Duration{time.Second, 2 * time.Second, 4 * time.Second, 8 * time.Second, 16 * time.Second, 30 * time.Second, 30 * time.Second, 30 * time.Second}
	if !slices.Equal(delays, want) {
		t.Fatalf("delays %v, want %v", delays, want)
	}
	backoff.reset()
	if backoff.delay() != 0 {
		t.Fatalf("a reset backoff is zero, got %v", backoff.delay())
	}
	backoff.transient()
	if backoff.delay() != time.Second {
		t.Fatalf("a reset backoff restarts at one second, got %v", backoff.delay())
	}
}

func TestRuntimeService_PreviewSweep_TheSweepIntervalIsOneSecond(t *testing.T) {
	t.Parallel()
	if previewSweepInterval != time.Second {
		t.Fatalf("the preview sweep interval is %v", previewSweepInterval)
	}
}

func TestRuntimeService_PacingTests_PacingStartsTheNextTickAtOnceWhileOuterInputMoves(t *testing.T) {
	t.Parallel()
	now := time.Now()
	cases := []struct {
		name                       string
		humanBytes, forwardedBytes int
	}{
		{"outer bytes read", 4096, 0},
		{"positive master write", 0, 2696},
		{"both", 1, 1},
	}
	for _, c := range cases {
		for _, next := range []opt[time.Time]{none[time.Time](), some(now.Add(time.Second))} {
			progress := transportProgress{humanBytes: c.humanBytes, forwardedBytes: c.forwardedBytes}
			if pause := tickSleep(progress, next, now); pause != 0 {
				t.Fatalf("%s, next deadline %v: pause %v", c.name, next, pause)
			}
		}
	}
}

func TestRuntimeService_PacingTests_PacingKeepsTheIdleSleepWithoutOuterInput(t *testing.T) {
	t.Parallel()
	now := time.Now()
	idle := transportProgress{}
	cases := []struct {
		name string
		next opt[time.Time]
		want time.Duration
	}{
		{"no observation deadline", none[time.Time](), 2 * time.Millisecond},
		{"a later observation deadline", some(now.Add(time.Second)), 2 * time.Millisecond},
		{"an earlier observation deadline still shortens the idle sleep", some(now.Add(500 * time.Microsecond)), 500 * time.Microsecond},
		{"a passed observation deadline", some(now.Add(-time.Millisecond)), 0},
	}
	for _, c := range cases {
		if pause := tickSleep(idle, c.next, now); pause != c.want {
			t.Fatalf("%s: pause %v, want %v", c.name, pause, c.want)
		}
	}
}
