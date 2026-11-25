package core

import (
	"bytes"
	"context"
	"encoding/binary"
	"encoding/json/jsontext"
	"errors"
	"fmt"
	"io"
	"math"
	"net"
	"os"
	"path/filepath"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"testing"
	"time"

	"github.com/creack/pty"
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

const (
	serviceDescriptorEnv = "CAFLEET_REVIEW_RUNNER_DESCRIPTOR"
	serviceServicesEnv   = "CAFLEET_REVIEW_TEST_SERVICES"
	serviceDatabaseEnv   = "CAFLEET_REVIEW_RUNNER_DATABASE_URL"
)

func wireFrame(t *testing.T, value jsontext.Value) []byte {
	t.Helper()
	body := wireBytes(value)
	if len(body) > maxControlFrame {
		t.Fatalf("a fixture frame body of %d bytes exceeds the cap", len(body))
	}
	return independentFrame(body)
}

// controlWire is a blocking control connection bounded by realTimeBound per exchange.
type controlWire struct {
	conn *net.UnixConn
}

func dialControlWire(path string) (*controlWire, error) {
	conn, err := net.DialUnix("unix", nil, &net.UnixAddr{Name: path, Net: "unix"})
	if err != nil {
		return nil, err
	}
	return &controlWire{conn: conn}, nil
}

func (w *controlWire) close() {
	_ = w.conn.Close()
}

func (w *controlWire) send(t *testing.T, value jsontext.Value) error {
	t.Helper()
	if err := w.conn.SetDeadline(time.Now().Add(realTimeBound)); err != nil {
		return err
	}
	_, err := w.conn.Write(wireFrame(t, value))
	return err
}

func (w *controlWire) exchange(t *testing.T, value jsontext.Value) (jsontext.Value, error) {
	t.Helper()
	if err := w.send(t, value); err != nil {
		return nil, err
	}
	header := make([]byte, 4)
	if _, err := io.ReadFull(w.conn, header); err != nil {
		return nil, err
	}
	length := binary.BigEndian.Uint32(header)
	if length == 0 || length > maxControlFrame {
		t.Fatalf("invalid reply length %d", length)
	}
	body := make([]byte, length)
	if _, err := io.ReadFull(w.conn, body); err != nil {
		return nil, err
	}
	reply, ok := jsonParse(string(body))
	if !ok {
		t.Fatalf("the reply %q is JSON", body)
	}
	if !jsonEqual(jsonIndex(reply, "protocol_version"), jsontext.Value("1")) {
		t.Fatalf("reply %s carries protocol_version 1", reply)
	}
	if _, hasResult := jsonLookup(reply, "result"); hasResult == jsonHas(t, reply, "error") {
		t.Fatalf("reply %s carries exactly one of result and error", reply)
	}
	return reply, nil
}

func closedConnection(err error) bool {
	return errors.Is(err, io.EOF) || errors.Is(err, io.ErrUnexpectedEOF) || errors.Is(err, syscall.ECONNRESET) ||
		errors.Is(err, syscall.EPIPE) || errors.Is(err, syscall.ENOTCONN)
}

// assertControlRejected requires an error reply or a closed connection.
func assertControlRejected(t *testing.T, reply jsontext.Value, err error) {
	t.Helper()
	if err != nil {
		if !closedConnection(err) {
			t.Fatalf("request rejection requires a reply or closed connection: %v", err)
		}
		return
	}
	if !jsonHas(t, reply, "error") {
		t.Fatalf("the request is rejected: %s", reply)
	}
}

func replyResult(t *testing.T, reply jsontext.Value, err error) jsontext.Value {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
	if jsonHas(t, reply, "error") {
		t.Fatalf("the reply succeeds: %s", reply)
	}
	return jsonAt(t, reply, "result")
}

// serviceChild is a re-executed runner process, collected with wait4 so a stop is observable.
type serviceChild struct {
	pid    int
	status opt[unix.WaitStatus]
}

func (c *serviceChild) tryWait() (unix.WaitStatus, bool, error) {
	if status, set := c.status.get(); set {
		return status, true, nil
	}
	for {
		var status unix.WaitStatus
		pid, err := unix.Wait4(c.pid, &status, unix.WNOHANG, nil)
		if errors.Is(err, unix.EINTR) {
			continue
		}
		if err != nil {
			return 0, false, err
		}
		if pid == 0 {
			return 0, false, nil
		}
		c.status = some(status)
		return status, true, nil
	}
}

func (c *serviceChild) pollUntil(until time.Time) (unix.WaitStatus, bool, error) {
	for {
		status, exited, err := c.tryWait()
		if err != nil || exited || !time.Now().Before(until) {
			return status, exited, err
		}
		time.Sleep(min(5*time.Millisecond, time.Until(until)))
	}
}

func (c *serviceChild) terminateUntil(until time.Time) {
	_ = unix.Kill(c.pid, unix.SIGKILL)
	_, _, _ = c.pollUntil(until)
}

func exitCode(status unix.WaitStatus) (int, bool) {
	if !status.Exited() {
		return 0, false
	}
	return status.ExitStatus(), true
}

// serviceFixture is a runner re-executed from this test binary over a private PTY, database and
// run, with the creator connection the launch path holds.
type serviceFixture struct {
	t               *testing.T
	fixture         *runnerFixture
	artifacts       *runArtifacts
	child           *serviceChild
	stderr          *os.File
	stderrFD        int
	process         opt[processIdentity]
	master          *os.File
	slave           *os.File
	masterIO        int
	creator         *controlWire
	prepared        *controlIdentity
	output          []byte
	stopped         bool
	cleanupDeadline opt[time.Time]
	startupDeadline time.Time
}

func (f *serviceFixture) path(name string) string {
	return filepath.Join(f.fixture.dir, name)
}

func (f *serviceFixture) assertSelectedConfig(path string) {
	f.t.Helper()
	data, err := os.ReadFile(f.path("config-selected.json"))
	if err != nil {
		f.t.Fatalf("explicit configured service selection record: %v", err)
	}
	selected := jsontext.Value(data)
	configured, err := os.ReadFile(path)
	if err != nil {
		f.t.Fatal(err)
	}
	if !jsonIsString(jsonIndex(selected, "configuration"), path) || !jsonIsString(jsonIndex(selected, "sha256"), sha256Hex(configured)) {
		f.t.Fatalf("selection record %s", selected)
	}
}

func (f *serviceFixture) connectionDiagnostic(stage string) string {
	status, exited, waitErr := f.child.tryWait()
	_, socketErr := os.Lstat(f.artifacts.paths.controlSocket)
	stderr := make([]byte, 64*1024)
	count, readErr := unix.Read(f.stderrFD, stderr)
	runState := "unreadable"
	if run, err := getRun(context.Background(), f.fixture.conn, f.fixture.run.runID); err == nil {
		runState = fmt.Sprintf("status=%s exit_code=%v cleanup=%s ended_at=%v diagnostics=%s", run.status.asStr(), run.exitCode, run.cleanupState.asStr(), run.endedAt, run.cleanupDiagnosticsJSON)
	}
	var kinds []any
	if rows, err := sqlRows(f.fixture.conn, "SELECT kind FROM member_runtime_events WHERE run_id=?1 ORDER BY event_seq", f.fixture.run.runID); err == nil {
		for _, row := range rows {
			kinds = append(kinds, row[0])
		}
	}
	outer := make([]byte, 64*1024)
	outerCount, _ := unix.Read(f.masterIO, outer)
	f.output = append(f.output, outer[:max(outerCount, 0)]...)
	tail := f.output[max(len(f.output)-2048, 0):]
	return fmt.Sprintf("stage=%s; runner_pid=%d; prepared=%v; exit=%v/%v/%v; socket=%v; run=%s; events=%v; stderr_read=%v; stderr=%q; outer_tail=%q",
		stage, f.child.pid, f.prepared != nil, status, exited, waitErr, socketErr, runState, kinds, readErr, stderr[:max(count, 0)], tail)
}

func (f *serviceFixture) connect(stage string) *controlWire {
	f.t.Helper()
	wire, err := dialControlWire(f.artifacts.paths.controlSocket)
	if err != nil {
		f.t.Fatalf("owned runner connection failed: %v; %s", err, f.connectionDiagnostic(stage))
	}
	f.t.Cleanup(wire.close)
	return wire
}

func newServiceFixture(t *testing.T, missingExecutable bool) *serviceFixture {
	t.Helper()
	return launchServiceFixture(t, missingExecutable, func(*testing.T, *runnerFixture, *spawnSpec) {}, false, none[time.Time]())
}

func configuredServiceFixture(t *testing.T, missingExecutable bool, configure func(*testing.T, *runnerFixture, *spawnSpec)) *serviceFixture {
	t.Helper()
	return launchServiceFixture(t, missingExecutable, configure, false, none[time.Time]())
}

const serviceNativeScript = `printf 'review-native-ready\n'; while IFS= read -r line; do case "$line" in finish) exit 0 ;; *) printf 'review-answer=%s\n' "$line" ;; esac; done`

func launchServiceFixture(t *testing.T, missingExecutable bool, configure func(*testing.T, *runnerFixture, *spawnSpec), refusal bool, callerDeadline opt[time.Time]) *serviceFixture {
	t.Helper()
	startupDeadline := time.Now().Add(realTimeBound)
	if caller, set := callerDeadline.get(); set && caller.Before(startupDeadline) {
		startupDeadline = caller
	}
	fixture := newRunnerFixture(t)
	if _, err := sqlExecute(fixture.conn, "UPDATE member_placements SET mux_pane_id=NULL, mux_window_id=NULL, mux_window_name=NULL WHERE member_id=?1", fixture.run.memberID); err != nil {
		t.Fatal(err)
	}
	artifacts := fixture.artifacts(t)
	spec := fixture.spec()
	if missingExecutable {
		spec.argv = []string{filepath.Join(fixture.dir, "missing-native-executable")}
	} else {
		spec.argv = []string{"/bin/sh", "-c", serviceNativeScript}
	}
	configure(t, fixture, &spec)
	descriptor, err := newLaunchDescriptor(fixture.run, spec)
	if err != nil {
		t.Fatal(err)
	}
	if err := artifacts.writeDescriptor(descriptor); err != nil {
		t.Fatal(err)
	}
	master, slave, err := pty.Open()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = master.Close(); _ = slave.Close() })
	if err := setWindowSize(master, terminalSize{rows: 24, cols: 80}); err != nil {
		t.Fatal(err)
	}
	masterIO, err := unix.FcntlInt(master.Fd(), unix.F_DUPFD_CLOEXEC, 0)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { closeFD(t, masterIO) })
	if err := unix.SetNonblock(masterIO, true); err != nil {
		t.Fatal(err)
	}
	stderrRead, stderrWrite, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = stderrRead.Close() })
	stderrFD := int(stderrRead.Fd())
	if err := unix.SetNonblock(stderrFD, true); err != nil {
		t.Fatal(err)
	}
	servicesConfig := filepath.Join(fixture.dir, "observation-services.json")
	_, statErr := os.Stat(servicesConfig)
	captureCommand, capture := captureChildCommand(t, fixture.dir)
	servicesSelected := statErr == nil && !capture
	command := childTestCommand(TestRuntimeService_ControlService_ServiceChild)
	if capture {
		command = captureCommand
	}
	command.Env = slices.DeleteFunc(command.Env, func(entry string) bool { return strings.HasPrefix(entry, serviceServicesEnv+"=") })
	if servicesSelected {
		command.Env = append(command.Env, serviceServicesEnv+"="+servicesConfig)
	}
	command.Env = append(command.Env,
		serviceDescriptorEnv+"="+artifacts.paths.descriptor,
		serviceDatabaseEnv+"="+databaseURLFor(fixture.databaseFile(t)))
	command.Stdin, command.Stdout, command.Stderr = slave, slave, stderrWrite
	if !time.Now().Before(startupDeadline) {
		t.Fatal("original fixture startup deadline")
	}
	if err := command.Start(); err != nil {
		t.Fatal(err)
	}
	_ = stderrWrite.Close()
	owned := &serviceFixture{
		t: t, fixture: fixture, artifacts: artifacts, child: &serviceChild{pid: command.Process.Pid}, stderr: stderrRead, stderrFD: stderrFD,
		master: master, slave: slave, masterIO: masterIO, cleanupDeadline: callerDeadline, startupDeadline: startupDeadline,
	}
	t.Cleanup(owned.cleanup)
	if refusal {
		if code, ok := exitCode(owned.wait()); ok && code == 0 {
			t.Fatal("nonfresh startup is refused")
		}
		if servicesSelected {
			owned.assertSelectedConfig(servicesConfig)
		}
		return owned
	}
	process, present, err := processIdentityRead(int64(owned.child.pid))
	if err != nil || !present {
		t.Fatalf("the runner's process identity: %v", err)
	}
	owned.process = some(process)
	for {
		if status, exited, err := owned.child.tryWait(); err != nil || exited {
			t.Fatalf("owned runner stopped before initial connection: %v %v; %s", status, err, owned.connectionDiagnostic("startup"))
		}
		if !time.Now().Before(startupDeadline) {
			t.Fatalf("owned runner did not accept its initial connection: %s", owned.connectionDiagnostic("startup timeout"))
		}
		wire, err := dialControlWire(artifacts.paths.controlSocket)
		if err == nil {
			owned.creator = wire
			break
		}
		if !errors.Is(err, syscall.ENOENT) && !errors.Is(err, syscall.ECONNREFUSED) {
			t.Fatalf("owned runner initial connection failed: %v; %s", err, owned.connectionDiagnostic("startup"))
		}
		time.Sleep(min(5*time.Millisecond, time.Until(startupDeadline)))
	}
	if servicesSelected {
		owned.assertSelectedConfig(servicesConfig)
	}
	return owned
}

func (f *serviceFixture) request(kind string, identity controlIdentity) jsontext.Value {
	f.t.Helper()
	return jsontext.Value(fmt.Sprintf(`{"protocol_version":1,"kind":%q,"identity":%s}`, kind, mustJSON(f.t, identity)))
}

func (f *serviceFixture) preparedIdentity() controlIdentity {
	f.t.Helper()
	if f.prepared == nil {
		f.t.Fatal("the runner is prepared")
	}
	return *f.prepared
}

func (f *serviceFixture) prepare() {
	f.t.Helper()
	if f.creator == nil {
		f.t.Fatal("connected initial creator stream")
	}
	request := jsonSetAt(f.t, f.request("prepare", controlIdentityFromRun(f.fixture.run)), "9000", "remaining_ms")
	reply, err := f.creator.exchange(f.t, request)
	result := replyResult(f.t, reply, err)
	var identity controlIdentity
	if err := decodeStrict(jsonAt(f.t, result, "identity"), &identity); err != nil {
		f.t.Fatal(err)
	}
	if identity.OwnerEpoch != 2 {
		f.t.Fatalf("the prepared owner epoch is 2, got %d", identity.OwnerEpoch)
	}
	if _, set := mustRun(f.t, f.fixture.conn, f.fixture.run.runID).childPID.get(); set {
		f.t.Fatal("a prepared run records no native child")
	}
	f.prepared = &identity
}

func (f *serviceFixture) publish() {
	f.t.Helper()
	for _, statement := range []string{
		"UPDATE member_placements SET mux_pane_id='%9', mux_window_id=COALESCE(mux_window_id, '@1'), mux_window_name=COALESCE(mux_window_name, 'main') WHERE member_id=?1",
	} {
		if _, err := sqlExecute(f.fixture.conn, statement, f.fixture.run.memberID); err != nil {
			f.t.Fatal(err)
		}
	}
	if _, err := sqlExecute(f.fixture.conn, "UPDATE member_runs SET status='prepared' WHERE run_id=?1", f.fixture.run.runID); err != nil {
		f.t.Fatal(err)
	}
}

func (f *serviceFixture) start() (jsontext.Value, error) {
	f.t.Helper()
	return f.creator.exchange(f.t, jsonSetAt(f.t, f.request("start", f.preparedIdentity()), `"%9"`, "pane_id"))
}

// startRunning prepares, publishes and starts the run, returning the started native child's wire
// identity once the native child is ready.
func (f *serviceFixture) startRunning() jsontext.Value {
	f.t.Helper()
	f.prepare()
	f.publish()
	reply, err := f.start()
	child := jsonAt(f.t, replyResult(f.t, reply, err), "child")
	f.readUntil("review-native-ready")
	return child
}

func (f *serviceFixture) health() jsontext.Value {
	f.t.Helper()
	started := time.Now()
	wire := f.connect("health")
	reply, err := wire.exchange(f.t, f.request("health", f.preparedIdentity()))
	wire.close()
	if err != nil {
		f.t.Fatalf("health exchange after %v: %v; %s", time.Since(started), err, f.connectionDiagnostic("health"))
	}
	return replyResult(f.t, reply, nil)
}

func (f *serviceFixture) healthChild() jsontext.Value {
	f.t.Helper()
	return jsonAt(f.t, f.health(), "child")
}

func (f *serviceFixture) readUntil(needle string) {
	f.t.Helper()
	until := time.Now().Add(realTimeBound)
	if needle == "review-native-ready" {
		until = f.startupDeadline
	}
	buffer := make([]byte, 4096)
	for {
		count, err := unix.Read(f.masterIO, buffer)
		switch {
		case err == nil:
			f.output = append(f.output, buffer[:count]...)
		case errors.Is(err, unix.EAGAIN), errors.Is(err, unix.EINTR):
		default:
			f.t.Fatalf("owned outer PTY read: %v", err)
		}
		if strings.Contains(lossyUTF8(f.output), needle) {
			return
		}
		if !time.Now().Before(until) {
			f.t.Fatalf("missing native output %s: %s; outer_bytes=%q", needle, f.connectionDiagnostic("native output timeout"), f.output)
		}
		if _, exited, _ := f.child.tryWait(); exited {
			f.t.Fatalf("runner stopped during manual I/O: %s", f.connectionDiagnostic("native output runner exit"))
		}
		time.Sleep(5 * time.Millisecond)
	}
}

func (f *serviceFixture) writeOuter(data string) {
	f.t.Helper()
	if err := writeFixtureBytes(f.masterIO, []byte(data), time.Now().Add(realTimeBound)); err != nil {
		f.t.Fatal(err)
	}
}

func (f *serviceFixture) assertManualIO() {
	f.t.Helper()
	f.writeOuter("literal '$x' `bytes`\r")
	f.readUntil("review-answer=literal '$x' `bytes`")
}

func (f *serviceFixture) wait() unix.WaitStatus {
	f.t.Helper()
	status, exited, err := f.child.pollUntil(time.Now().Add(realTimeBound))
	if err != nil {
		f.t.Fatal(err)
	}
	if !exited {
		f.t.Fatal("owned runner exceeded bounded cleanup")
	}
	return status
}

func (f *serviceFixture) assertExitCode(want int) {
	f.t.Helper()
	if code, ok := exitCode(f.wait()); !ok || code != want {
		f.t.Fatalf("the runner exits %d, got %d (exited %v); %s", want, code, ok, f.connectionDiagnostic("exit"))
	}
}

func (f *serviceFixture) finish() {
	f.t.Helper()
	f.writeOuter("finish\r")
	f.assertExitCode(0)
	run := mustRun(f.t, f.fixture.conn, f.fixture.run.runID)
	if run.status != runStatusExited || run.exitCode != some[int64](0) || run.cleanupState != cleanupStateComplete {
		f.t.Fatalf("run %s exit %v cleanup %s: %s", run.status.asStr(), run.exitCode, run.cleanupState.asStr(), run.cleanupDiagnosticsJSON)
	}
}

func (f *serviceFixture) stopForDisconnect() {
	f.t.Helper()
	if process, _ := f.process.get(); !mustMatch(f.t, process) {
		f.t.Fatal("the runner keeps its generation")
	}
	if err := unix.Kill(f.child.pid, unix.SIGSTOP); err != nil {
		f.t.Fatal(err)
	}
	f.stopped = true
	until := time.Now().Add(realTimeBound)
	for {
		var status unix.WaitStatus
		pid, err := unix.Wait4(f.child.pid, &status, unix.WUNTRACED|unix.WNOHANG, nil)
		switch {
		case errors.Is(err, unix.EINTR):
		case err != nil:
			f.t.Fatal(err)
		case pid != 0 && stoppedBySigstop(status):
			return
		case pid != 0:
			f.t.Fatalf("unexpected owned runner state: %v", status)
		}
		if !time.Now().Before(until) {
			f.t.Fatal("owned runner did not stop")
		}
		time.Sleep(2 * time.Millisecond)
	}
}

// stoppedBySigstop decodes the wait status bits directly: the BSD WaitStatus methods report a
// SIGSTOP stop as Continued rather than Stopped.
func stoppedBySigstop(status unix.WaitStatus) bool {
	return status&0xff == 0x7f && unix.Signal(status>>8&0xff) == unix.SIGSTOP
}

func mustMatch(t *testing.T, process processIdentity) bool {
	t.Helper()
	matches, err := process.matches()
	if err != nil {
		t.Fatal(err)
	}
	return matches
}

func (f *serviceFixture) resume() {
	f.t.Helper()
	if process, _ := f.process.get(); !mustMatch(f.t, process) {
		f.t.Fatal("the runner keeps its generation")
	}
	if err := unix.Kill(f.child.pid, unix.SIGCONT); err != nil {
		f.t.Fatal(err)
	}
	f.stopped = false
}

func (f *serviceFixture) cleanupRecordedNative() {
	run, err := getRun(context.Background(), f.fixture.conn, f.fixture.run.runID)
	if err != nil {
		f.t.Logf("owned fixture cleanup cannot inspect its run: %v", err)
		return
	}
	pid, hasPID := run.childPID.get()
	start, hasStart := run.childStartIdentity.get()
	if !hasPID || !hasStart {
		return
	}
	owned := processIdentity{pid: pid, startIdentity: start}
	matches, err := owned.matches()
	if err != nil {
		f.t.Logf("owned native fixture identity is unconfirmed: %v", err)
		return
	}
	if matches {
		if err := unix.Kill(int(pid), unix.SIGKILL); err != nil {
			f.t.Logf("owned native fixture termination failed: %v", err)
		}
	}
}

func (f *serviceFixture) cleanup() {
	if f.creator != nil {
		f.creator.close()
		f.creator = nil
	}
	if f.stopped {
		_ = unix.Kill(f.child.pid, unix.SIGCONT)
		f.stopped = false
	}
	if _, exited, err := f.child.tryWait(); exited || err != nil {
		f.cleanupRecordedNative()
		return
	}
	_ = unix.Kill(f.child.pid, unix.SIGTERM)
	until, set := f.cleanupDeadline.get()
	if !set {
		until = time.Now().Add(realTimeBound)
	}
	if _, exited, err := f.child.pollUntil(until.Add(-time.Second)); exited || err != nil {
		f.cleanupRecordedNative()
		return
	}
	f.cleanupRecordedNative()
	f.child.terminateUntil(until)
}

func TestRuntimeService_ControlService_ServiceChild(t *testing.T) {
	t.Parallel()
	skipUnlessChildTest(t)
	if config, ok := os.LookupEnv(serviceInputNativeEnv); ok {
		runServiceInputNative(config)
		os.Exit(0)
	}
	path, err := databasePath(os.Getenv(serviceDatabaseEnv))
	if err != nil {
		fmt.Fprintf(os.Stderr, "service child runner failed: %v\n", err)
		os.Exit(1)
	}
	selected := settingsForDatabase(path)
	descriptor := os.Getenv(serviceDescriptorEnv)
	var status int32
	if configured, ok := os.LookupEnv(serviceServicesEnv); ok {
		if !filepath.IsAbs(configured) {
			fmt.Fprintln(os.Stderr, "explicit service configuration is absolute")
			os.Exit(1)
		}
		status, err = runConfiguredServices(selected, descriptor, configured)
	} else {
		status, err = runnerRun(selected, descriptor)
	}
	if err != nil {
		fmt.Fprintf(os.Stderr, "service child runner failed: %v\n", err)
		os.Exit(1)
	}
	os.Exit(int(status))
}

// serviceWriterHooks fail or block the event writer at boundaries that private files in the
// service configuration's directory select.
type serviceWriterHooks struct {
	directory string
}

func (h serviceWriterHooks) exists(name string) bool {
	_, err := os.Stat(filepath.Join(h.directory, name))
	return err == nil
}

func (h serviceWriterHooks) before(boundary writerBoundary, bound *stepBound) error {
	if boundary == writerBoundaryFlush && h.exists("fail-flush") {
		return databaseBusyError("owned synthetic flush busy")
	}
	if boundary != writerBoundaryPersist {
		return nil
	}
	if h.exists("fail-persist") {
		return databaseBusyError("owned synthetic persist busy")
	}
	entered, err := os.OpenFile(filepath.Join(h.directory, "persist-entered"), os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	switch {
	case err == nil:
		_ = entered.Close()
	case errors.Is(err, os.ErrExist):
	default:
		return ioError(ioErrorText(err))
	}
	for h.exists("block-persist") {
		if err := bound.check(); err != nil {
			return err
		}
		time.Sleep(2 * time.Millisecond)
	}
	return nil
}

// runConfiguredServices runs the runner with the test services the configuration file selects.
func runConfiguredServices(s settings, descriptor, config string) (int32, error) {
	data, err := os.ReadFile(config)
	if err != nil {
		return 0, ioError(ioErrorText(err))
	}
	value := parseTestJSON(string(data))
	catalog, err := loadTestCatalog(jget(value, "catalog"))
	if err != nil {
		return 0, err
	}
	directory := filepath.Dir(config)
	if err := os.WriteFile(filepath.Join(directory, "config-selected.json"), jsonBytes(jo("configuration", config, "sha256", sha256Hex(data))), 0o600); err != nil {
		return 0, ioError(ioErrorText(err))
	}
	diagnostics := newRunnerDiagnostics()
	if seeded, ok := ju64(jget(value, "seed_diagnostics")); ok {
		stamp := wallStamp(systemClock{})
		for index := range seeded {
			diagnostics.record(runnerDiagnosticSourceQueueHousekeeping, fmt.Sprintf("label: seeded %02d", index), stamp)
		}
	}
	hooks, err := injectedObservationHooks(value, directory)
	if err != nil {
		return 0, err
	}
	previewSweep := false
	if enabled, ok := jget(value, "preview_sweep").(bool); ok {
		previewSweep = enabled
	}
	return runService(s, descriptor, runnerDependencies{
		catalog:          catalog,
		clock:            systemClock{},
		diagnostics:      diagnostics,
		writerHooks:      serviceWriterHooks{directory: directory},
		observationHooks: hooks,
		previewSweep:     previewSweep,
	})
}

// privateFixtureWrite creates path with mode 0600 holding data, staged and hard-linked so a
// reader never sees a partial file; an existing path is an error.
func privateFixtureWrite(t *testing.T, path string, data []byte) {
	t.Helper()
	staged := filepath.Join(filepath.Dir(path), fmt.Sprintf(".%s.partial-%d-%d", filepath.Base(path), os.Getpid(), stagedWrites.Add(1)))
	file, err := os.OpenFile(staged, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := file.Write(data); err != nil {
		t.Fatal(err)
	}
	if err := file.Sync(); err != nil {
		t.Fatal(err)
	}
	if err := file.Close(); err != nil {
		t.Fatal(err)
	}
	if err := os.Link(staged, path); err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(staged); err != nil {
		t.Fatal(err)
	}
}

var stagedWrites atomic.Uint64

const syntheticObservationScript = "#!/bin/sh\nif [ \"$1\" = --version ]; then printf '0.0.1-synthetic\\n'; exit 0; fi\n/usr/bin/touch \"$CAFLEET_REVIEW_NATIVE_START\"\nprintf 'review-native-ready\\n'\nwhile IFS= read -r line; do case \"$line\" in finish) exit 0 ;; form*) printf '\\033[2J\\033[19;1HSynthetic permission request\\r\\nOperation: write %s.txt\\r\\n> Allow once\\r\\n  Deny once\\r\\nRequest form: synthetic-only\\r\\nSelection: ' \"$line\" ;; transient) printf '\\033[2J\\033[19;1HSynthetic permission request\\r\\nOperation: write transient.txt\\r\\n> Allow once\\r\\n  Deny once\\r\\nRequest form: synthetic-only\\r\\nSelection: \\033[2J\\033[Hworking' ;; *) printf 'review-answer=%s\\n' \"$line\" ;; esac; done\n"

// syntheticObservationServices runs a scripted native child under the synthetic catalog and a
// tested coverage, with the services configuration selecting that catalog.
func syntheticObservationServices(t *testing.T, fixture *runnerFixture, spec *spawnSpec) {
	t.Helper()
	spec.environment = append(spec.environment, [2]string{"CAFLEET_REVIEW_NATIVE_START", filepath.Join(fixture.dir, "native-started")})
	if _, err := sqlExecute(fixture.conn, "UPDATE member_runs SET native_version='0.0.1-synthetic',coverage='tested' WHERE run_id=?1", fixture.run.runID); err != nil {
		t.Fatal(err)
	}
	fixture.refresh(t)
	script := filepath.Join(fixture.dir, "owned-observation-process")
	privateFixtureWrite(t, script, []byte(syntheticObservationScript))
	if err := os.Chmod(script, 0o700); err != nil {
		t.Fatal(err)
	}
	spec.argv = []string{script, "--no-daemon", "--approve-for-me"}
	privateFixtureWrite(t, filepath.Join(fixture.dir, "observation-services.json"), jsonBytes(jo("catalog", syntheticDefinitions())))
}

const failingRunningTrigger = "CREATE TRIGGER reject_review_running BEFORE UPDATE OF status ON member_runs WHEN NEW.status='running' BEGIN SELECT RAISE(ABORT,'fixture running persistence blocked'); END"

func TestRuntimeService_ControlService_ControlInvalidProtocolVersionsLeaveTheRunningChildAvailable(t *testing.T) {
	t.Parallel()
	f := newServiceFixture(t, false)
	started := f.startRunning()
	for _, version := range []string{"", "2", `"1"`, "1.0"} {
		request := f.request("health", f.preparedIdentity())
		if version == "" {
			request = testJSONWithout(t, request, "protocol_version")
		} else {
			request = jsonSetAt(t, request, version, "protocol_version")
		}
		reply, err := f.connect("invalid protocol").exchange(t, request)
		assertControlRejected(t, reply, err)
		assertJSONEqual(t, f.healthChild(), started, "version "+version)
	}
	for _, stale := range []string{"nonce", "epoch"} {
		identity := f.preparedIdentity()
		if stale == "nonce" {
			identity.LaunchNonce = strings.Repeat("b", 64)
		} else {
			identity.OwnerEpoch++
		}
		request := jsonSetAt(t, f.request("start", identity), `"%9"`, "pane_id")
		reply, err := f.connect("stale start").exchange(t, request)
		assertControlRejected(t, reply, err)
		assertJSONEqual(t, f.healthChild(), started, stale)
	}
	reply, err := f.start()
	assertControlRejected(t, reply, err)
	assertJSONEqual(t, f.healthChild(), started, "repeated start")
	f.assertManualIO()
	f.finish()
}

func TestRuntimeService_ControlService_ControlNoncreatorStartIsIsolatedBeforeAndAfterSpawn(t *testing.T) {
	t.Parallel()
	f := newServiceFixture(t, false)
	f.prepare()
	f.publish()
	request := jsonSetAt(t, f.request("start", f.preparedIdentity()), `"%9"`, "pane_id")
	reply, err := f.connect("noncreator before start").exchange(t, request)
	assertControlRejected(t, reply, err)
	if !jsonIsNull(f.healthChild()) {
		t.Fatal("a noncreator start spawns nothing")
	}
	reply, err = f.start()
	started := jsonAt(t, replyResult(t, reply, err), "child")
	f.readUntil("review-native-ready")
	reply, err = f.connect("noncreator after start").exchange(t, request)
	assertControlRejected(t, reply, err)
	assertJSONEqual(t, f.healthChild(), started, "after a noncreator start")
	f.assertManualIO()
	f.finish()
}

func TestRuntimeService_ControlService_ControlDisconnectedReplyPreservesHealthAndManualIo(t *testing.T) {
	t.Parallel()
	f := newServiceFixture(t, false)
	started := f.startRunning()
	f.stopForDisconnect()
	disconnected := f.connect("disconnected health peer")
	if err := disconnected.send(t, f.request("health", f.preparedIdentity())); err != nil {
		t.Fatal(err)
	}
	disconnected.close()
	f.resume()
	assertJSONEqual(t, f.healthChild(), started, "after a disconnected peer")
	f.assertManualIO()
	f.finish()
}

func TestRuntimeService_ControlService_ControlCreatorDisconnectBeforeStartExitsWithoutSpawning(t *testing.T) {
	t.Parallel()
	f := newServiceFixture(t, false)
	f.prepare()
	f.creator.close()
	f.creator = nil
	f.assertExitCode(1)
	run := mustRun(t, f.fixture.conn, f.fixture.run.runID)
	if _, set := run.childPID.get(); set || run.status != runStatusFailed {
		t.Fatalf("run %s child %v", run.status.asStr(), run.childPID)
	}
	if _, set := run.endedAt.get(); !set {
		t.Fatal("the failed run records its end")
	}
	assertRunLockReleased(t, f.fixture.conn, run)
}

func assertRunLockReleased(t *testing.T, conn *dbConn, run runRecord) {
	t.Helper()
	released, err := runLockAcquireReleased(context.Background(), conn, run.owner())
	if err != nil {
		t.Fatal(err)
	}
	if lock, set := released.get(); !set {
		t.Fatal("the runner released its run lock")
	} else if err := lock.close(); err != nil {
		t.Fatal(err)
	}
}

func TestRuntimeService_ControlService_ControlAcceptedStartSpawnAndStorageFailuresCompensate(t *testing.T) {
	t.Parallel()
	for _, storageFailure := range []bool{false, true} {
		f := newServiceFixture(t, !storageFailure)
		f.prepare()
		f.publish()
		if storageFailure {
			mustExecBatch(t, f.fixture.conn, failingRunningTrigger)
		}
		reply, err := f.start()
		assertControlRejected(t, reply, err)
		f.assertExitCode(1)
		run := mustRun(t, f.fixture.conn, f.fixture.run.runID)
		if _, set := run.endedAt.get(); run.status != runStatusFailed || !set {
			t.Fatalf("storage failure %v: run %s", storageFailure, run.status.asStr())
		}
		reported := fmt.Sprintf("%s %v %s", reply, err, run.cleanupDiagnosticsJSON)
		if storageFailure && !strings.Contains(reported, "fixture running persistence blocked") {
			t.Fatalf("the storage failure is reported: %s", reported)
		}
		if _, err := os.Stat(f.artifacts.paths.descriptor); !os.IsNotExist(err) {
			t.Fatalf("storage failure %v: the descriptor is removed: %v", storageFailure, err)
		}
		assertRunLockReleased(t, f.fixture.conn, run)
	}
}

// jsonStrings is the JSON array of strings value.
func jsonStrings(t *testing.T, value jsontext.Value) []string {
	t.Helper()
	items, ok := jsonArray(value)
	if !ok {
		t.Fatalf("%s is a JSON array", value)
	}
	texts := []string{}
	for _, item := range items {
		text, ok := jsonString(item)
		if !ok {
			t.Fatalf("%s holds only strings", value)
		}
		texts = append(texts, text)
	}
	return texts
}

type runnerDiagnosticRow struct {
	source  string
	message string
	count   int64
}

func runnerDiagnosticRows(t *testing.T, conn *dbConn, runID int64, sourceFilter string) []runnerDiagnosticRow {
	t.Helper()
	sql := "SELECT source,message,count FROM member_runner_diagnostics WHERE run_id=?1 ORDER BY diagnostic_id"
	if sourceFilter != "" {
		sql = "SELECT source,message,count FROM member_runner_diagnostics WHERE run_id=?1 AND source='" + sourceFilter + "' ORDER BY diagnostic_id"
	}
	var rows []runnerDiagnosticRow
	for _, row := range tableRows(t, conn, sql, runID) {
		rows = append(rows, runnerDiagnosticRow{source: row[0].(string), message: row[1].(string), count: row[2].(int64)})
	}
	return rows
}

func TestRuntimeService_ControlService_ControlShutdownOutcomeDiagnosticsAreStoredAsShutdownRows(t *testing.T) {
	t.Parallel()
	f := newServiceFixture(t, false)
	f.prepare()
	f.publish()
	mustExecBatch(t, f.fixture.conn, failingRunningTrigger)
	reply, err := f.start()
	assertControlRejected(t, reply, err)
	f.assertExitCode(1)
	run := mustRun(t, f.fixture.conn, f.fixture.run.runID)
	outcome := jsonStrings(t, run.cleanupDiagnosticsJSON)
	if len(outcome) == 0 {
		t.Fatal("the failed start reports its cause")
	}
	rows := runnerDiagnosticRows(t, f.fixture.conn, run.runID, "shutdown")
	for _, diagnostic := range outcome {
		sanitized := diagnosticText(diagnostic)
		if !slices.ContainsFunc(rows, func(row runnerDiagnosticRow) bool { return strings.Contains(row.message, sanitized) && row.count > 0 }) {
			t.Fatalf("%q has a shutdown row: %v", diagnostic, rows)
		}
	}
}

const seededDiagnostics = 40

func seededUnflushable(t *testing.T, fixture *runnerFixture, spec *spawnSpec) {
	t.Helper()
	syntheticObservationServices(t, fixture, spec)
	path := filepath.Join(fixture.dir, "observation-services.json")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	config := parseTestJSON(string(data))
	jset(config, uint64(seededDiagnostics), "seed_diagnostics")
	if err := os.WriteFile(path, jsonBytes(config), 0o600); err != nil {
		t.Fatal(err)
	}
	privateFixtureWrite(t, filepath.Join(fixture.dir, "fail-flush"), []byte("owned test gate"))
}

func TestRuntimeService_ControlService_ControlShutdownWritesStillBufferedDiagnosticsAndTheOverflowRow(t *testing.T) {
	t.Parallel()
	f := configuredServiceFixture(t, false, seededUnflushable)
	f.prepare()
	f.publish()
	mustExecBatch(t, f.fixture.conn, failingRunningTrigger)
	reply, err := f.start()
	assertControlRejected(t, reply, err)
	f.assertExitCode(1)
	rows := runnerDiagnosticRows(t, f.fixture.conn, f.fixture.run.runID, "")
	seeded := func(index int) string { return fmt.Sprintf("label: seeded %02d", index) }
	if !slices.ContainsFunc(rows, func(row runnerDiagnosticRow) bool {
		return row.source == "queue_housekeeping" && row.message == seeded(seededDiagnostics-1)
	}) {
		t.Fatalf("the newest buffered entry is written at shutdown: %v", rows)
	}
	for evicted := range 8 {
		if slices.ContainsFunc(rows, func(row runnerDiagnosticRow) bool { return row.message == seeded(evicted) }) {
			t.Fatalf("seeded entry %d was evicted from the 32-entry buffer: %v", evicted, rows)
		}
	}
	var overflow []uint64
	for _, row := range rows {
		if row.source != "event_writer" {
			continue
		}
		count, found := strings.CutPrefix(row.message, "diagnostic buffer overflow: ")
		count, trimmed := strings.CutSuffix(count, " entries dropped")
		if dropped, err := strconv.ParseUint(count, 10, 64); found && trimmed && err == nil {
			overflow = append(overflow, dropped)
		}
	}
	if len(overflow) != 1 || overflow[0] < 8 {
		t.Fatalf("one overflow row of at least 8 entries: %v", rows)
	}
	if !slices.ContainsFunc(rows, func(row runnerDiagnosticRow) bool { return row.source == "shutdown" }) {
		t.Fatalf("the outcome diagnostics are written alongside: %v", rows)
	}
	if len(rows) > 256 {
		t.Fatalf("at most 256 diagnostic rows, got %d", len(rows))
	}
}

// scenarioShutdownUnit is every scenario's shutdown unit, so a storage deadline lies 400 ms of
// virtual time out.
const (
	scenarioShutdownUnit = 100 * time.Millisecond
	scenarioWriterWait   = realTimeBound
	scenarioTurnLimit    = 100_000
)

type scenarioStepKind uint8

const (
	scenarioScreen scenarioStepKind = iota + 1
	scenarioType
	scenarioRun
	scenarioPrompt
	scenarioExec
	scenarioReconcile
	scenarioMessage
	scenarioConsumed
	scenarioHoldPane
	scenarioReleasePane
	scenarioArm
	scenarioFailPersist
)

// scenarioStep is one scripted step: the native child draws a bundled screen, the human types
// bytes, the runner turns for a virtual duration, a control client submits or reconciles, the
// Director sends a message, another writer holds the pane lock, a storage fault is armed, or every
// event writer persist fails while set.
type scenarioStep struct {
	kind     scenarioStepKind
	screen   string
	bytes    string
	duration time.Duration
	key      string
	literal  string
	body     string
	plan     faultPlan
	failing  bool
}

func stepScreen(id string) scenarioStep  { return scenarioStep{kind: scenarioScreen, screen: id} }
func stepType(bytes string) scenarioStep { return scenarioStep{kind: scenarioType, bytes: bytes} }
func stepRun(duration time.Duration) scenarioStep {
	return scenarioStep{kind: scenarioRun, duration: duration}
}
func stepPrompt(key, literal string) scenarioStep {
	return scenarioStep{kind: scenarioPrompt, key: key, literal: literal}
}
func stepExec(key, literal string) scenarioStep {
	return scenarioStep{kind: scenarioExec, key: key, literal: literal}
}
func stepReconcile(key string) scenarioStep { return scenarioStep{kind: scenarioReconcile, key: key} }
func stepMessage(body string) scenarioStep  { return scenarioStep{kind: scenarioMessage, body: body} }
func stepConsumed(body string) scenarioStep { return scenarioStep{kind: scenarioConsumed, body: body} }
func stepArm(plan faultPlan) scenarioStep   { return scenarioStep{kind: scenarioArm, plan: plan} }
func stepFailPersist(failing bool) scenarioStep {
	return scenarioStep{kind: scenarioFailPersist, failing: failing}
}

var (
	stepHoldPane    = scenarioStep{kind: scenarioHoldPane}
	stepReleasePane = scenarioStep{kind: scenarioReleasePane}
)

// scenarioInterrupt makes the native child draw screen once, as soon as the bytes written to it
// contain marker.
type scenarioInterrupt struct {
	marker string
	screen string
}

// scenario is a scenario as data: the native screen the child starts on, the screen it redraws
// after each submit key, the screen it draws once its input contains a marker, the steps, and the
// runtime event kinds the run records, in order.
type scenario struct {
	name      string
	screen    string
	redraw    opt[string]
	interrupt opt[scenarioInterrupt]
	steps     []scenarioStep
	events    []string
}

// run runs every step and asserts the recorded event kinds, returning the finished runner for
// further row and byte assertions.
func (s scenario) run(t *testing.T) *scenarioRunner {
	t.Helper()
	return s.runWith(t, func(*scenarioRunner) {})
}

func (s scenario) runWith(t *testing.T, configure func(*scenarioRunner)) *scenarioRunner {
	t.Helper()
	runner := newScenarioRunner(t, s.screen, s.redraw)
	if interrupt, set := s.interrupt.get(); set {
		runner.child.interrupt = &scenarioChildInterrupt{marker: []byte(interrupt.marker), screen: scenarioScreenBytes(t, interrupt.screen)}
	}
	configure(runner)
	runner.settle()
	for _, step := range s.steps {
		runner.step(step)
	}
	runner.settle()
	if kinds := runner.eventKinds(); !slices.Equal(kinds, s.events) {
		t.Fatalf("%s: events %q, want %q; delivery results %s, child received %q",
			s.name, kinds, s.events, runner.payloads("input_delivery_result"), runner.child.written)
	}
	return runner
}

// scenarioWriterHooks fails every event writer persist with a transient busy cause while set.
type scenarioWriterHooks struct {
	failPersist atomic.Bool
}

func (h *scenarioWriterHooks) before(boundary writerBoundary, _ *stepBound) error {
	if boundary == writerBoundaryPersist && h.failPersist.Load() {
		return databaseBusyError("scenario persist busy")
	}
	return nil
}

type scenarioChildInterrupt struct {
	marker []byte
	screen []byte
}

type scenarioChildRedraw struct {
	submit []byte
	screen []byte
}

// scenarioNativeChild is the native child behind the PTY: it draws queued screens, records every
// byte the runner writes to it, paints the text written since the last submit key into its
// composer, and draws the redraw screen after each write that ends with its submit key.
type scenarioNativeChild struct {
	screens   *fakeInput
	written   []byte
	redraw    *scenarioChildRedraw
	interrupt *scenarioChildInterrupt
	painter   composerPainter
	typed     []byte
	// programmatic reports whether the runner's input coordinator, rather than the human, is
	// writing.
	programmatic bool
}

func (c *scenarioNativeChild) draw(t *testing.T, screen []byte) {
	t.Helper()
	if len(screen) == 0 {
		t.Fatal("an empty read is end of file")
	}
	c.screens.push(screen)
}

// echo adds the printable text of data to the composer; escape sequences such as a lone Escape
// or a pointer report paint nothing, and paste markers frame text.
func (c *scenarioNativeChild) echo(data []byte) {
	if c.painter == nil {
		return
	}
	text := slices.Clone(data)
	for _, marker := range [][]byte{[]byte("\x1b[200~"), []byte("\x1b[201~")} {
		text = bytes.ReplaceAll(text, marker, nil)
	}
	if len(text) == 0 || text[0] == 0x1b || (!c.programmatic && len(c.typed) == 0) {
		return
	}
	for _, b := range text {
		if b >= 0x20 && b != 0x7f {
			c.typed = append(c.typed, b)
		}
	}
	c.screens.push(c.painter(lossyUTF8(c.typed)))
}

func (c *scenarioNativeChild) tryRead(target []byte) (int, error) {
	return c.screens.tryRead(target)
}

func (c *scenarioNativeChild) tryWrite(data []byte) (int, error) {
	c.written = append(c.written, data...)
	switch {
	case c.interrupt != nil && bytes.Contains(c.written, c.interrupt.marker):
		c.screens.push(c.interrupt.screen)
		c.interrupt = nil
	case c.redraw != nil && bytes.HasSuffix(c.written, c.redraw.submit):
		c.typed = nil
		c.screens.push(c.redraw.screen)
	default:
		c.echo(data)
	}
	return len(data), nil
}

func (c *scenarioNativeChild) resize(terminalSize) error { return nil }
func (c *scenarioNativeChild) close() error              { return nil }

func scenarioScreenBytes(t *testing.T, id string) []byte {
	t.Helper()
	return nativeFixtureBytes(bundledScreen(id))
}

type scenarioNativeScreen struct {
	backend codingAgent
	version string
	size    terminalSize
}

func scenarioNative(t *testing.T, id string) scenarioNativeScreen {
	t.Helper()
	backend, file := bundledFile(id)
	screen := jget(file, "screens", id)
	dimension := func(key string) uint16 {
		value, ok := ju64(jget(screen, key))
		if !ok || value > math.MaxUint16 {
			t.Fatalf("screen %s has a u16 %s", id, key)
		}
		return uint16(value) //nolint:gosec // bounded by the check above
	}
	version, ok := jstring(jget(screen, "native_version"))
	if !ok {
		t.Fatalf("screen %s names its native version", id)
	}
	return scenarioNativeScreen{backend: backend, version: version, size: terminalSize{rows: dimension("rows"), cols: dimension("cols")}}
}

// scenarioIo is the runner I/O the production input turns drive: the scenario's fake terminal,
// scripted child and observation, with no termination, resize or native exit.
type scenarioIo struct {
	runner *scenarioRunner
}

func (io scenarioIo) clock() clock { return io.runner.clock }

func (io scenarioIo) controller() *observationController { return io.runner.controller }

func (io scenarioIo) quiesce(coordinator *inputCoordinator, deadline time.Time) (inputDrain, *inputFailure) {
	r := io.runner
	return progressWithManual(coordinator, r.transport, func(t *transport) (inputDrain, error) {
		return t.quiesceForInput(r.human, r.child, r.display, r.controller.observer(r.writer, r.clock), deadline)
	})
}

func (io scenarioIo) advance(ctx context.Context, coordinator *inputCoordinator, conn *dbConn, deadline time.Time) (opt[inputControlReply], error) {
	r := io.runner
	r.child.programmatic = true
	defer func() { r.child.programmatic = false }()
	reply, failure := coordinator.advance(ctx, conn, r.transport, r.child, r.controller, deadline)
	if failure != nil {
		return none[inputControlReply](), failure.asError()
	}
	return reply, nil
}

func (scenarioIo) terminateRequested() bool { return false }
func (scenarioIo) resizeRequested() bool    { return false }

func (io scenarioIo) shutdownDeadlines() shutdownDeadlines {
	return shutdownDeadlinesStartingNow(io.runner.clock)
}

func (scenarioIo) observeExit(context.Context, *inputCoordinator, *dbConn) (opt[shutdownDeadlines], error) {
	return none[shutdownDeadlines](), nil
}

type scenarioReply struct {
	reply inputControlReply
	cause *cafleetError
}

// scenarioRunner is the runtime scenario harness: the runner's transport, observation, event
// writer and input coordinator over a scripted native child, fake terminal and temporary
// database, driven in the runner loop's turn order on one virtual testClock.
type scenarioRunner struct {
	t           *testing.T
	clock       *testClock
	fixture     *writerFixture
	writer      *eventWriter
	controller  *observationController
	transport   *transport
	coordinator *inputCoordinator
	human       *fakeInput
	child       *scenarioNativeChild
	display     *fakeDisplay
	replies     []scenarioReply
	heldPane    *paneLock
	hooks       *scenarioWriterHooks
	turns       *inputTurns
	finished    bool
}

// newScenarioRunner is a started run of the backend and version that recorded screen, at its
// size, with the child showing screen.
func newScenarioRunner(t *testing.T, screen string, redraw opt[string]) *scenarioRunner {
	t.Helper()
	ctx := context.Background()
	catalog := profileCatalogBundled()
	native := scenarioNative(t, screen)
	c := virtualClock().withShutdownUnit(scenarioShutdownUnit)
	fixture := newWriterFixture(t)
	coverage := catalog.capabilities(native.backend).detection
	if _, err := sqlExecute(fixture.conn, "UPDATE member_runs SET backend=?1,native_version=?2,coverage=?3 WHERE run_id=?4",
		native.backend.name(), native.version, coverage.asStr(), fixture.run.runID); err != nil {
		t.Fatal(err)
	}
	if _, err := sqlExecute(fixture.conn, "UPDATE member_placements SET coding_agent=?1 WHERE member_id=?2", native.backend.name(), fixture.run.memberID); err != nil {
		t.Fatal(err)
	}
	fixture.refresh(t)
	controller, err := newObservationController(fixture.run, catalog)
	if err != nil {
		t.Fatal(err)
	}
	hooks := &scenarioWriterHooks{}
	writer, err := eventWriterStartWorker(fixture.url, fixture.run, fixture.lock, yieldingClock{c}, newRunnerDiagnostics(), hooks)
	if err != nil {
		t.Fatal(err)
	}
	child := currentProcess(t)
	if _, err := sqlExecute(fixture.conn, "UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at=?3 WHERE run_id=?4",
		child.pid, child.startIdentity, wallStamp(c), fixture.run.runID); err != nil {
		t.Fatal(err)
	}
	fixture.refresh(t)
	tr, err := newTransport(native.size, transportLimits{inputBytes: 65536, displayBytes: 262144, bytesPerTick: 16384}, c)
	if err != nil {
		t.Fatal(err)
	}
	snapshot := tr.snapshot()
	if err := controller.start(fixture.run, &snapshot, writer, c); err != nil {
		t.Fatal(err)
	}
	coordinator, err := newInputCoordinator(ctx, fixture.conn, fixture.run, fixture.lock,
		newInputServices(catalog, systemRequestTokens{}, c), c.now().Add(10*time.Second))
	if err != nil {
		t.Fatal(err)
	}
	var childRedraw *scenarioChildRedraw
	if id, set := redraw.get(); set {
		plan, ok := catalog.composerPlan(native.backend, composerOperationPrompt)
		if !ok {
			t.Fatal("a redraw after submit needs the profile's prompt plan")
		}
		stages, err := plan.stages("x", literalFramingPlain)
		if err != nil {
			t.Fatal(err)
		}
		childRedraw = &scenarioChildRedraw{submit: stages[len(stages)-1], screen: scenarioScreenBytes(t, id)}
	}
	painter, _ := nativeComposerPainter(t, native.backend, native.size, scenarioScreenBytes(t, screen))
	runner := &scenarioRunner{
		t: t, clock: c, fixture: fixture, writer: writer, controller: controller, transport: tr,
		coordinator: coordinator, human: &fakeInput{}, display: &fakeDisplay{}, hooks: hooks, turns: newInputTurns(),
		child: &scenarioNativeChild{screens: &fakeInput{}, redraw: childRedraw, painter: painter},
	}
	runner.child.draw(t, scenarioScreenBytes(t, screen))
	t.Cleanup(func() {
		if !runner.finished && !t.Failed() {
			runner.finish()
		}
		if runner.heldPane != nil {
			runner.heldPane.close()
		}
	})
	return runner
}

func (r *scenarioRunner) step(step scenarioStep) {
	r.t.Helper()
	switch step.kind {
	case scenarioScreen:
		r.child.draw(r.t, scenarioScreenBytes(r.t, step.screen))
	case scenarioType:
		if step.bytes == "" {
			r.t.Fatal("an empty read is end of file")
		}
		r.human.push([]byte(step.bytes))
	case scenarioRun:
		r.runFor(step.duration)
	case scenarioPrompt:
		r.submit(submitPromptOperation{input: submitInput{OperationKey: step.key, InputKind: inputKindPrompt, Literal: step.literal}})
	case scenarioExec:
		r.submit(submitExecOperation{input: submitInput{OperationKey: step.key, InputKind: inputKindExec, Literal: step.literal}})
	case scenarioReconcile:
		found, err := findInput(context.Background(), r.fixture.conn, r.fixture.run.runID, step.key)
		if err != nil {
			r.t.Fatal(err)
		}
		input, ok := found.get()
		if !ok {
			r.t.Fatalf("%s has a receipt", step.key)
		}
		r.submit(receiptOperation{action: receiptReconcile{InputID: input.inputID, Expected: inputStateUncertain, Disposition: inputDispositionSubmitted, EvidenceRef: "audit:pane-shows-the-input"}})
	case scenarioMessage:
		r.send(step.body)
	case scenarioConsumed:
		message := r.send(step.body)
		if _, err := sqlExecute(r.fixture.conn, "UPDATE messages SET status_state='completed' WHERE message_id=?1", message.messageID); err != nil {
			r.t.Fatal(err)
		}
	case scenarioHoldPane:
		if r.heldPane != nil {
			r.t.Fatal("one held pane lock")
		}
		held, acquired, err := paneLockTryAcquire(r.fixture.url, runMemberPlacement(r.t, r.fixture.conn, r.fixture.run))
		if err != nil || !acquired {
			r.t.Fatalf("the member's pane input lock is free: %v", err)
		}
		r.heldPane = held
	case scenarioReleasePane:
		if r.heldPane == nil {
			r.t.Fatal("a held pane lock")
		}
		r.heldPane.close()
		r.heldPane = nil
	case scenarioArm:
		step.plan.arm(r.t, r.fixture.conn)
	case scenarioFailPersist:
		r.hooks.failPersist.Store(step.failing)
	default:
		panic("invalid scenarioStepKind")
	}
}

// settle turns until every queued screen and typed byte is consumed and nothing more moves.
func (r *scenarioRunner) settle() {
	r.t.Helper()
	for range scenarioTurnLimit {
		start := r.clock.elapsed()
		r.turn()
		if len(r.child.screens.chunks) == 0 && len(r.human.chunks) == 0 && r.turns.idle() && r.clock.elapsed() > start {
			return
		}
	}
	r.t.Fatalf("the scenario settles within %d turns", scenarioTurnLimit)
}

func (r *scenarioRunner) runFor(duration time.Duration) {
	r.t.Helper()
	until := r.clock.elapsed() + duration
	for range scenarioTurnLimit {
		if r.clock.elapsed() >= until {
			return
		}
		r.skipIdleTurns(r.clock.now().Add(until - r.clock.elapsed()))
		r.turn()
	}
	r.t.Fatalf("%v of virtual time passes within %d turns", duration, scenarioTurnLimit)
}

// turn is one runner loop turn of a started runner: the transport tick and observation poll, the
// production input turns, then the loop's idle sleep.
func (r *scenarioRunner) turn() {
	r.t.Helper()
	r.awaitWriter()
	progress, failure := progressWithManual(r.coordinator, r.transport, func(t *transport) (transportProgress, error) {
		return t.tick(r.human, r.child, r.display, r.controller.observer(r.writer, r.clock))
	})
	if failure != nil {
		r.t.Fatal(failure.asError())
	}
	snapshot := r.transport.snapshot()
	if _, err := r.controller.poll(&snapshot, r.writer, r.clock); err != nil {
		r.t.Fatal(err)
	}
	outcome, err := r.turns.turn(context.Background(), r.coordinator, r.fixture.conn, r.fixture.run, scenarioIo{runner: r})
	if err != nil {
		r.t.Fatal(err)
	}
	switch outcome := outcome.(type) {
	case inputTurnContinue:
	case inputTurnReply:
		r.replies = append(r.replies, scenarioReply{reply: outcome.reply, cause: outcome.cause})
	case inputTurnShutdown:
		r.t.Fatal("a scenario runner requests no shutdown")
	default:
		r.t.Fatalf("turn outcome %T", outcome)
	}
	r.awaitWriter()
	if pause := tickSleep(progress, r.controller.status().nextDeadline, r.clock.now()); pause > 0 {
		r.clock.sleep(pause)
	}
}

// skipIdleTurns moves the clock to the next turn that can do work, or to limit, while nothing is
// in flight: the turns in between read no bytes and reach no timer.
func (r *scenarioRunner) skipIdleTurns(limit time.Time) {
	idle := r.turns.idle() && !r.coordinator.hasDeferredWork() && len(r.child.screens.chunks) == 0 &&
		len(r.human.chunks) == 0 && r.transport.pendingInputBytes() == 0 && !r.controller.hasPending()
	if !idle {
		return
	}
	now := r.clock.now()
	wake := limit
	wakes := r.turns.wakes(now)
	for _, candidate := range append(wakes[:], r.controller.status().nextDeadline) {
		if at, set := candidate.get(); set && at.Before(wake) {
			wake = at
		}
	}
	if wake.After(now) {
		r.clock.advance(wake.Sub(now))
	}
}

// awaitWriter waits in real time until the writer has persisted every accepted event or reported
// a failure, so each turn observes a settled sink.
func (r *scenarioRunner) awaitWriter() {
	r.t.Helper()
	until := time.Now().Add(scenarioWriterWait)
	for {
		status := r.writer.status()
		if _, failed := status.diagnostic.get(); status.pendingCount == 0 || failed {
			return
		}
		if !time.Now().Before(until) {
			r.t.Fatalf("the event writer settles: %+v", status)
		}
		runtime.Gosched()
	}
}

// submit admits a control request the way the runner serves an input client.
func (r *scenarioRunner) submit(operation inputControlOperation) {
	r.t.Helper()
	expiration, err := inputExpirationCapture(r.clock.now().Add(30*time.Second), r.clock)
	if err != nil {
		r.t.Fatal(err)
	}
	request := inputControlRequest{Identity: controlIdentityFromRun(r.fixture.run), ExpiresAtMonoNS: expiration.monotonicNs(), Operation: operation}
	deadline, err := inputDeadline(request, r.clock.now(), r.clock)
	if err != nil {
		r.t.Fatal(err)
	}
	reply, failure := r.coordinator.handleReceived(context.Background(), r.fixture.conn, request, r.controller, deadline)
	switch {
	case failure != nil:
		r.replies = append(r.replies, scenarioReply{cause: failure.asError()})
	case reply.set:
		value, _ := reply.get()
		r.replies = append(r.replies, scenarioReply{reply: value})
	default:
		if err := r.turns.admitClient(0, deadline); err != nil {
			r.t.Fatal(err)
		}
	}
}

func (r *scenarioRunner) send(body string) messageRecord {
	r.t.Helper()
	message, err := sendMessage(context.Background(), r.fixture.conn, r.fixture.run.directorMemberID, r.fixture.run.memberID, body)
	if err != nil {
		r.t.Fatal(err)
	}
	return message
}

// previewLine is the preview line the sweep types for the message with this body.
func (r *scenarioRunner) previewLine(body string) string {
	r.t.Helper()
	return canonicalPreviewLine(r.message(body))
}

func (r *scenarioRunner) message(body string) messageRecord {
	r.t.Helper()
	id := queryInt64(r.t, r.fixture.conn, "SELECT message_id FROM messages WHERE text=?1", body)
	message, err := getMessage(context.Background(), r.fixture.conn, id)
	if err != nil {
		r.t.Fatal(err)
	}
	return message
}

// typed is how many times the child received text.
func (r *scenarioRunner) typed(text string) int {
	return strings.Count(lossyUTF8(r.child.written), text)
}

// typedAt is the byte offset of the first text the child received.
func (r *scenarioRunner) typedAt(text string) int {
	r.t.Helper()
	at := strings.Index(lossyUTF8(r.child.written), text)
	if at < 0 {
		r.t.Fatalf("the child received %q", text)
	}
	return at
}

func (r *scenarioRunner) notified(body string) bool {
	r.t.Helper()
	return queryInt64(r.t, r.fixture.conn, "SELECT notified_at IS NOT NULL FROM messages WHERE text=?1", body) == 1
}

func (r *scenarioRunner) payloads(kind string) []jsontext.Value {
	r.t.Helper()
	var payloads []jsontext.Value
	for _, row := range tableRows(r.t, r.fixture.conn, "SELECT payload_json FROM member_runtime_events WHERE run_id=?1 AND kind=?2 ORDER BY event_seq", r.fixture.run.runID, kind) {
		payloads = append(payloads, jsontext.Value(row[0].(string)))
	}
	return payloads
}

func (r *scenarioRunner) eventKinds() []string {
	r.t.Helper()
	kinds := []string{}
	for _, row := range tableRows(r.t, r.fixture.conn, "SELECT kind FROM member_runtime_events WHERE run_id=?1 ORDER BY event_seq", r.fixture.run.runID) {
		kinds = append(kinds, row[0].(string))
	}
	return kinds
}

// sweepBackoffUntil is when the run-wide sweep backoff next allows a sweep, once a failed attempt
// armed it.
func (r *scenarioRunner) sweepBackoffUntil() opt[time.Time] {
	return r.turns.previewSweep.backoffUntil
}

func (r *scenarioRunner) eligibility() inputEligibility {
	return r.controller.health(r.writer).Eligibility
}

// finish stops the writer at the shutdown storage deadline and returns its unsaved pending and
// gap counts; a writer that cannot persist is abandoned at that deadline.
func (r *scenarioRunner) finish() (uint64, uint64) {
	r.finished = true
	r.awaitWriter()
	until := shutdownDeadlinesStartingNow(r.clock).storageDeadline()
	r.writer.beginShutdown(until)
	if r.writer.status().pendingCount > 0 {
		r.clock.advance(max(until.Sub(r.clock.now()), 0))
	}
	return r.writer.finish(until)
}

func (r *scenarioRunner) assertFinished(label string) {
	r.t.Helper()
	if pending, gap := r.finish(); pending != 0 || gap != 0 {
		r.t.Fatalf("%s: the writer finishes with pending %d and gap %d", label, pending, gap)
	}
}

func (r *scenarioRunner) submission(index int) submissionResult {
	r.t.Helper()
	if index < 0 {
		index += len(r.replies)
	}
	reply := r.replies[index]
	if reply.cause != nil {
		r.t.Fatalf("a submission reply, got %v", reply.cause)
	}
	submission, ok := reply.reply.(submissionResult)
	if !ok {
		r.t.Fatalf("a submission reply, got %T", reply.reply)
	}
	return submission
}

const (
	scenarioIdle     = "codex-0.167.0-composer-idle-empty-tmux-120-1"
	scenarioApproval = "codex-0.167.0-allow-pre-stage-tmux-120-1"
	scenarioStatus   = "codex-0.167.0-auto-status-tmux-120-1"
	scenarioDraft    = "codex-0.167.0-composer-prompt-plain-41-literal-tmux-120-1"
	// scenarioSweepWindow is longer than two sweep intervals, so a sweep that could deliver would
	// have done so.
	scenarioSweepWindow = 2500 * time.Millisecond
)

var (
	scenarioRejectOpening = faultPlanAbort("scenario_reject_opening",
		"BEFORE INSERT ON member_runtime_events WHEN NEW.kind='interaction_opened'", "scenario opening rejection")
	scenarioMidWrite = scenarioInterrupt{marker: "FORM_MID_WRITE", screen: scenarioApproval}
)

func scenarioPromptProgram(t *testing.T, literal string) []byte {
	t.Helper()
	plan, ok := profileCatalogBundled().composerPlan(codingAgentCodex, composerOperationPrompt)
	if !ok {
		t.Fatal("codex declares a prompt plan")
	}
	stages, err := plan.stages(literal, literalFramingPlain)
	if err != nil {
		t.Fatal(err)
	}
	return slices.Concat(stages[:]...)
}

func TestRuntimeService_Scenarios_ScreenScenariosRecordTheirEventSequences(t *testing.T) {
	t.Parallel()
	scenarios := []scenario{
		{name: "an idle composer records nothing", screen: scenarioIdle, steps: []scenarioStep{stepRun(2 * time.Second)}, events: []string{}},
		{
			name: "an approval form opens an interaction that a later screen closes", screen: scenarioStatus,
			steps:  []scenarioStep{stepScreen(scenarioApproval), stepRun(time.Second), stepScreen(scenarioStatus), stepRun(time.Second)},
			events: []string{"interaction_opened", "interaction_closed"},
		},
	}
	for _, s := range scenarios {
		t.Run(s.name, func(t *testing.T) {
			t.Parallel()
			s.run(t).assertFinished(s.name)
		})
	}
}

func TestRuntimeService_Scenarios_HumanTypingReachesTheChildAndTheChildScreenReachesTheDisplay(t *testing.T) {
	t.Parallel()
	runner := scenario{name: "typing", screen: scenarioIdle, steps: []scenarioStep{stepType("typed by hand")}, events: []string{}}.run(t)
	if string(runner.child.written) != "typed by hand" {
		t.Fatalf("the child received %q", runner.child.written)
	}
	if !bytes.Equal(runner.display.bytes, scenarioScreenBytes(t, scenarioIdle)) {
		t.Fatalf("the display shows %q", runner.display.bytes)
	}
	runner.assertFinished("typing")
}

func TestRuntimeService_Scenarios_APromptIntoAVerifiedEmptyComposerWritesItsPlanAndIsAccepted(t *testing.T) {
	t.Parallel()
	const literal = "scenario prompt"
	runner := scenario{
		name: "prompt", screen: scenarioIdle, redraw: some(scenarioIdle),
		steps:  []scenarioStep{stepPrompt("scenario:prompt", literal), stepRun(5 * time.Second)},
		events: []string{},
	}.run(t)
	if !bytes.Equal(runner.child.written, scenarioPromptProgram(t, literal)) {
		t.Fatalf("the child received %q", runner.child.written)
	}
	if len(runner.replies) != 1 {
		t.Fatalf("one submission reply, got %d", len(runner.replies))
	}
	if reply := runner.submission(0); reply.Status != submissionStatusAccepted || reply.Stage != submissionStageSubmitted {
		t.Fatalf("reply %+v", reply)
	}
	var slept time.Duration
	for _, sleep := range runner.clock.sleeps() {
		slept += sleep
	}
	if slept < stageSettle {
		t.Fatalf("the staged write settles on the virtual clock: %v", slept)
	}
	runner.assertFinished("prompt")
}

func TestRuntimeService_Scenarios_TheSweepTypesEachPendingPreviewOnceOldestFirstAndStampsIt(t *testing.T) {
	t.Parallel()
	runner := scenario{
		name: "oldest first", screen: scenarioIdle, redraw: some(scenarioIdle),
		steps: []scenarioStep{
			stepConsumed("consumed before any sweep"), stepMessage("first swept preview"),
			stepMessage("second swept preview"), stepRun(3 * time.Second),
		},
		events: []string{},
	}.run(t)
	first, second := runner.previewLine("first swept preview"), runner.previewLine("second swept preview")
	if runner.typed(first) != 1 || runner.typed(second) != 1 || runner.typedAt(first) >= runner.typedAt(second) {
		t.Fatalf("each preview once, oldest first: %q", runner.child.written)
	}
	if !runner.notified("first swept preview") || !runner.notified("second swept preview") {
		t.Fatal("each typed preview is stamped")
	}
	if runner.typed("consumed before any sweep") != 0 || runner.notified("consumed before any sweep") {
		t.Fatal("a consumed message is never typed")
	}
	runner.assertFinished("oldest first")
}

func TestRuntimeService_Scenarios_AnObstacleDefersTheSweepWithZeroBytesUntilItClears(t *testing.T) {
	t.Parallel()
	cases := []scenario{
		{
			name: "a non-empty composer blocks delivery until it verifies empty", screen: scenarioIdle, redraw: some(scenarioIdle),
			steps:  []scenarioStep{stepScreen(scenarioDraft), stepRun(100 * time.Millisecond), stepMessage("blocked preview"), stepRun(scenarioSweepWindow)},
			events: []string{},
		},
		{
			name: "a held pane lock defers the sweep until it is free", screen: scenarioIdle, redraw: some(scenarioIdle),
			steps:  []scenarioStep{stepHoldPane, stepMessage("blocked preview"), stepRun(scenarioSweepWindow)},
			events: []string{},
		},
	}
	for _, s := range cases {
		t.Run(s.name, func(t *testing.T) {
			t.Parallel()
			runner := s.run(t)
			if len(runner.child.written) != 0 || runner.notified("blocked preview") {
				t.Fatalf("%s: zero bytes while blocked: %q", s.name, runner.child.written)
			}
			if runner.heldPane != nil {
				runner.step(stepReleasePane)
			} else {
				runner.step(stepScreen(scenarioIdle))
			}
			runner.step(stepRun(10 * time.Second))
			if line := runner.previewLine("blocked preview"); runner.typed(line) != 1 || !runner.notified("blocked preview") {
				t.Fatalf("%s: the cleared obstacle delivers once: %q", s.name, runner.child.written)
			}
			runner.assertFinished(s.name)
		})
	}
}

func TestRuntimeService_Scenarios_ARetriedHeadWaitsOutTheRunWideBackoffAndPrecedesANewerMessage(t *testing.T) {
	t.Parallel()
	runner := scenario{
		name: "retried head", screen: scenarioIdle, redraw: some(scenarioIdle),
		steps: []scenarioStep{
			stepHoldPane, stepMessage("head behind a busy pane"), stepMessage("newer message"),
			stepRun(scenarioSweepWindow), stepReleasePane,
		},
		events: []string{},
	}.run(t)
	released := runner.clock.now()
	backoff, armed := runner.sweepBackoffUntil().get()
	if !armed {
		t.Fatal("a pane_input_busy hold arms the run-wide backoff")
	}
	if backoff.Before(released) {
		t.Fatalf("the backoff outlasts the release: %v", backoff.Sub(released))
	}
	runner.step(stepRun(max(backoff.Sub(released)-10*time.Millisecond, 0)))
	if len(runner.child.written) != 0 {
		t.Fatal("no preview byte is written while the backoff is armed")
	}
	runner.step(stepRun(10 * time.Second))
	head, newer := runner.previewLine("head behind a busy pane"), runner.previewLine("newer message")
	if runner.typed(head) != 1 || runner.typed(newer) != 1 || runner.typedAt(head) >= runner.typedAt(newer) {
		t.Fatalf("the head precedes the newer message: %q", runner.child.written)
	}
	if len(runner.payloads("input_delivery_result")) != 0 {
		t.Fatal("a held and retried preview publishes no delivery result")
	}
	runner.assertFinished("retried head")
}

func TestRuntimeService_Scenarios_AControlPromptQueuesBehindAnInFlightSweepOrAnOpenInteraction(t *testing.T) {
	t.Parallel()
	cases := []scenario{
		{
			name: "a prompt arriving during a sweep attempt queues behind it", screen: scenarioIdle, redraw: some(scenarioIdle),
			steps:  []scenarioStep{stepMessage("ahead of the prompt"), stepRun(1250 * time.Millisecond), stepPrompt("prompt:queued", "queued control prompt")},
			events: []string{},
		},
		{
			name: "a prompt queued behind a form is written before the sweep", screen: scenarioIdle, redraw: some(scenarioIdle),
			steps: []scenarioStep{
				stepScreen(scenarioApproval), stepRun(time.Second), stepPrompt("prompt:queued", "queued control prompt"),
				stepMessage("behind the prompt"), stepRun(scenarioSweepWindow),
			},
			events: []string{"interaction_opened"},
		},
	}
	for index, s := range cases {
		t.Run(s.name, func(t *testing.T) {
			t.Parallel()
			runner := s.run(t)
			if len(runner.replies) == 0 || runner.submission(-1).Status != submissionStatusQueued {
				t.Fatalf("%s: the prompt queues", s.name)
			}
			if runner.typed("queued control prompt") != 0 {
				t.Fatalf("%s: the queued prompt waits", s.name)
			}
			if index == 1 {
				runner.step(stepScreen(scenarioIdle))
			}
			runner.step(stepRun(5 * time.Second))
			var first, second string
			if index == 0 {
				first, second = runner.previewLine("ahead of the prompt"), "queued control prompt"
			} else {
				first, second = "queued control prompt", runner.previewLine("behind the prompt")
			}
			if runner.typed(first) != 1 || runner.typed(second) != 1 || runner.typedAt(first) >= runner.typedAt(second) {
				t.Fatalf("%s: order %q", s.name, runner.child.written)
			}
			runner.finish()
		})
	}
}

func TestRuntimeService_Scenarios_AnUncertainPreviewPublishesOneReceiptFreeResultAndIsTypedAgainOnceVerifiedEmpty(t *testing.T) {
	t.Parallel()
	const body = "retried preview FORM_MID_WRITE"
	runner := scenario{
		name: "uncertain preview", screen: scenarioIdle, redraw: some(scenarioIdle), interrupt: some(scenarioMidWrite),
		steps:  []scenarioStep{stepMessage(body), stepRun(3 * time.Second)},
		events: []string{"input_delivery_result", "interaction_opened"},
	}.run(t)
	line := runner.previewLine(body)
	messageID := runner.message(body).messageID
	results := func() []jsontext.Value {
		var matching []jsontext.Value
		for _, result := range runner.payloads("input_delivery_result") {
			if id, ok := jsonInt64(jsonIndex(result, "message_id")); ok && id == messageID {
				matching = append(matching, result)
			}
		}
		return matching
	}
	published := results()
	if len(published) != 1 {
		t.Fatalf("one delivery result: %s", published)
	}
	if !jsonIsNull(jsonIndex(published[0], "input_id")) || !jsonIsString(jsonAt(t, published[0], "delivery", "status"), "uncertain") ||
		!jsonIsNull(jsonIndex(published[0], "director_action")) {
		t.Fatalf("a receipt-free uncertain result: %s", published[0])
	}
	if runner.notified(body) {
		t.Fatal("an uncertain preview is not stamped")
	}
	runner.step(stepScreen(scenarioIdle))
	runner.step(stepRun(5 * time.Second))
	if runner.typed(line) != 2 || !runner.notified(body) {
		t.Fatalf("the stopped literal and its retry: %d", runner.typed(line))
	}
	if len(results()) != 1 {
		t.Fatal("the retry publishes no further result")
	}
	runner.finish()
}

func TestRuntimeService_Scenarios_APreviewBehindANonEmptyComposerPublishesOneStallEventAtTheStallBound(t *testing.T) {
	t.Parallel()
	const bound = 2 * time.Second
	const body = "stalled behind a human draft"
	runner := scenario{
		name: "stalled preview", screen: scenarioIdle, redraw: some(scenarioIdle),
		steps:  []scenarioStep{stepScreen(scenarioDraft), stepRun(100 * time.Millisecond), stepMessage(body), stepRun(time.Second)},
		events: []string{},
	}.runWith(t, func(r *scenarioRunner) { r.coordinator.stallBound = bound })
	if len(runner.payloads("input_delivery_stalled")) != 0 {
		t.Fatal("no stall before the bound")
	}
	runner.step(stepRun(bound + scenarioSweepWindow))
	stalls := runner.payloads("input_delivery_stalled")
	if len(stalls) != 1 {
		t.Fatalf("published once per run and message: %s", stalls)
	}
	stall := stalls[0]
	if id, _ := jsonInt64(jsonIndex(stall, "message_id")); id != runner.message(body).messageID {
		t.Fatalf("the stall names its message: %s", stall)
	}
	if !jsonIsNull(jsonIndex(stall, "blocked_by_input_id")) {
		t.Fatalf("no blocking input: %s", stall)
	}
	if _, ok := jsonString(jsonIndex(stall, "waiting_since")); !ok {
		t.Fatalf("the stall names its wait start: %s", stall)
	}
	if instruction, ok := jsonString(jsonAt(t, stall, "director_action", "instruction")); !ok || strings.TrimSpace(instruction) == "" {
		t.Fatalf("one instruction: %s", stall)
	}
	if len(runner.child.written) != 0 {
		t.Fatal("the stalled preview writes nothing")
	}
	runner.step(stepScreen(scenarioIdle))
	runner.step(stepRun(5 * time.Second))
	if !runner.notified(body) || len(runner.payloads("input_delivery_stalled")) != 1 {
		t.Fatal("the cleared composer delivers without another stall")
	}
	runner.finish()
}

func TestRuntimeService_Scenarios_AHeadThatStallsAgainAfterAnInteractionArchivesTheRepeatAndLaterEventsPersist(t *testing.T) {
	t.Parallel()
	const bound = 2 * time.Second
	cycle := []scenarioStep{
		stepHoldPane, stepScreen(scenarioIdle), stepRun(time.Second), stepScreen(scenarioDraft),
		stepRun(500 * time.Millisecond), stepReleasePane, stepRun(4 * time.Second),
		stepScreen(scenarioApproval), stepRun(time.Second),
	}
	steps := slices.Concat([]scenarioStep{
		stepScreen(scenarioApproval), stepRun(time.Second), stepPrompt("prompt:restarted-stall", "head that stalls twice"),
	}, cycle, cycle)
	runner := scenario{
		name: "restarted head stall", screen: scenarioIdle, redraw: some(scenarioIdle), steps: steps,
		events: []string{"interaction_opened", "interaction_closed", "input_delivery_stalled", "interaction_opened", "interaction_closed", "input_delivery_stalled", "interaction_opened"},
	}.runWith(t, func(r *scenarioRunner) { r.coordinator.stallBound = bound })
	if runner.typed("head that stalls twice") != 0 {
		t.Fatal("the stalled head is never typed")
	}
	found, err := findInput(context.Background(), runner.fixture.conn, runner.fixture.run.runID, "prompt:restarted-stall")
	if err != nil {
		t.Fatal(err)
	}
	head, ok := found.get()
	if !ok {
		t.Fatal("the queued head has a receipt")
	}
	rows := tableRows(t, runner.fixture.conn, "SELECT notification_state,message_id,archive_reason,payload_json FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_stalled' ORDER BY event_seq", runner.fixture.run.runID)
	if len(rows) != 2 {
		t.Fatalf("two stalled records, got %v", rows)
	}
	first, repeat := rows[0], rows[1]
	if first[0] != "inbox" || first[1] == nil || first[2] != nil {
		t.Fatalf("the first stall reaches the inbox: %v", first)
	}
	if repeat[0] != "archived" || repeat[1] != nil || repeat[2] != "duplicate_delivery" {
		t.Fatalf("the repeat is archived as a duplicate: %v", repeat)
	}
	for _, row := range rows {
		payload := jsontext.Value(row[3].(string))
		if id, _ := jsonInt64(jsonIndex(payload, "input_id")); id != head.inputID {
			t.Fatalf("the stall names the head: %s", payload)
		}
	}
	if jsonEqual(jsonIndex(jsontext.Value(first[3].(string)), "waiting_since"), jsonIndex(jsontext.Value(repeat[3].(string)), "waiting_since")) {
		t.Fatalf("the restarted watch carries its own wait start: %v", rows)
	}
	if diagnostic, failed := runner.writer.status().diagnostic.get(); failed {
		t.Fatalf("later events persist: %s", diagnostic)
	}
	runner.finish()
}

func TestRuntimeService_Scenarios_AnUncertainPromptStopsQueuedInputAndTheSweepUntilReconciled(t *testing.T) {
	t.Parallel()
	const preview = "preview behind the uncertain prompt"
	runner := scenario{
		name: "uncertain prompt", screen: scenarioIdle, redraw: some(scenarioIdle), interrupt: some(scenarioMidWrite),
		steps: []scenarioStep{
			stepPrompt("prompt:stopped-mid-write", "stopped prompt FORM_MID_WRITE"), stepRun(3 * time.Second),
			stepScreen(scenarioIdle), stepRun(time.Second), stepExec("exec:behind-the-uncertain-prompt", "printf behind"),
			stepMessage(preview), stepRun(scenarioSweepWindow),
		},
		events: []string{"input_delivery_result", "interaction_opened", "interaction_closed"},
	}.run(t)
	if runner.submission(0).Status != submissionStatusUncertain || runner.submission(1).Status != submissionStatusQueued {
		t.Fatalf("replies %+v", runner.replies)
	}
	line := runner.previewLine(preview)
	if runner.typed("printf behind") != 0 || runner.typed(line) != 0 || runner.notified(preview) {
		t.Fatal("the uncertain prompt stops queued input and the sweep")
	}
	runner.step(stepReconcile("prompt:stopped-mid-write"))
	runner.step(stepRun(10 * time.Second))
	if runner.typed("printf behind") != 1 || runner.typed(line) != 1 {
		t.Fatalf("the reconciled run resumes: %q", runner.child.written)
	}
	if runner.typedAt("printf behind") >= runner.typedAt(line) {
		t.Fatal("the FIFO drains before the sweep")
	}
	runner.finish()
}

func TestRuntimeService_Scenarios_AnEventStorageFaultHoldsInputUntilTheWriterPersistsAgain(t *testing.T) {
	t.Parallel()
	runner := scenario{
		name: "event fault", screen: scenarioStatus,
		steps:  []scenarioStep{stepFailPersist(true), stepScreen(scenarioApproval), stepRun(time.Second)},
		events: []string{},
	}.run(t)
	if _, failed := runner.writer.status().diagnostic.get(); !failed {
		t.Fatal("the failing writer reports a diagnostic")
	}
	if eligibility := runner.eligibility(); eligibility != inputEligibility(inputEligibilityHold{reason: gateReasonGap}) {
		t.Fatalf("eligibility %v, want a gap hold", eligibility)
	}
	runner.step(stepFailPersist(false))
	runner.step(stepRun(5 * time.Second))
	if !slices.Contains(runner.eventKinds(), "interaction_opened") {
		t.Fatalf("the writer persists again: %q", runner.eventKinds())
	}
	runner.finish()
}

func TestRuntimeService_Scenarios_ARejectedEventDegradesWithoutAGapAndTheHeldPreviewIsDelivered(t *testing.T) {
	t.Parallel()
	const body = "preview held by the rejected form"
	runner := scenario{
		name: "rejected opening", screen: scenarioIdle, redraw: some(scenarioIdle),
		steps: []scenarioStep{
			stepArm(scenarioRejectOpening), stepScreen(scenarioApproval), stepRun(time.Second),
			stepMessage(body), stepRun(time.Second), stepScreen(scenarioIdle), stepRun(5 * time.Second),
		},
		events: []string{"runner_degraded", "runner_degraded"},
	}.run(t)
	line := runner.previewLine(body)
	var rejected []string
	for _, payload := range runner.payloads("runner_degraded") {
		if !jsonIsString(jsonAt(t, payload, "cause", "kind"), "event_rejected") {
			t.Fatalf("an event_rejected degradation: %s", payload)
		}
		kind, _ := jsonString(jsonAt(t, payload, "cause", "rejected_kind"))
		rejected = append(rejected, kind)
	}
	if !slices.Equal(rejected, []string{"interaction_opened", "interaction_closed"}) {
		t.Fatalf("rejected kinds %q", rejected)
	}
	status := runner.writer.status()
	if _, failed := status.diagnostic.get(); failed || status.gapCount != 0 || runner.controller.status().gapCount != 0 {
		t.Fatalf("a rejection is no gap: %+v", status)
	}
	if runner.typed(line) != 1 || !runner.notified(body) {
		t.Fatal("the held preview is typed once")
	}
	runner.finish()
}

// inputTurnsWithoutSweep is turns whose sweep never comes due, so a runner fixture's own preview
// delivery races no sweep.
func inputTurnsWithoutSweep() *inputTurns {
	turns := newInputTurns()
	turns.previewSweep = previewSweep{disabled: true}
	return turns
}

// idle reports whether no immediate attempt is in flight.
func (t *inputTurns) idle() bool {
	_, inFlight := t.pending.get()
	return !inFlight
}

// wakes is the earliest instants at which a turn has timer work: the next sweep pass, after any
// backoff, and the next queue housekeeping.
func (t *inputTurns) wakes(now time.Time) [2]opt[time.Time] {
	sweep, scheduled := t.previewSweep.nextAt.get()
	if until, backingOff := t.previewSweep.backoffUntil.get(); backingOff && (!scheduled || until.After(sweep)) {
		sweep, scheduled = until, true
	}
	if !scheduled {
		sweep = now
	}
	return [2]opt[time.Time]{some(sweep), t.queueHousekeepingAt}
}

const (
	serviceInputNativeEnv = "CAFLEET_REVIEW_SERVICE_INPUT_NATIVE"
	serviceInputScreen    = "\x1b[2J\x1b[23;1HSynthetic state\r\nState end"
	// serviceInputFormLine makes the native child show a detected permission form instead of
	// journaling it; the next journaled line restores the composer screen.
	serviceInputFormLine = "show-permission-form"
	serviceInputForm     = "\x1b[2J\x1b[19;1HSynthetic permission request\r\nOperation: write held.txt\r\n> Allow once\r\n  Deny once\r\nRequest form: synthetic-only\r\nSelection: "
	// serviceInputDraftLine makes the native child show a composer holding a draft, which no
	// composer rule matches; the next journaled line restores the composer screen.
	serviceInputDraftLine = "show-composer-draft"
	serviceInputDraft     = "\x1b[2J\x1b[23;1HSynthetic draft: half-typed\r\nState end"
	// serviceInputFormMidWrite makes the native child show the detected permission form while the
	// literal is still unsubmitted, once per native process, so the write stops mid-stage.
	serviceInputFormMidWrite = "open-form-mid-write"
	serviceInputLifetime     = 2 * realTimeBound
)

func serviceInputExpiration(t *testing.T, until time.Time) uint64 {
	t.Helper()
	expiration, err := inputExpirationCapture(until, systemClock{})
	if err != nil {
		t.Fatal(err)
	}
	return expiration.monotonicNs()
}

func serviceInputPause(t *testing.T, until time.Time) {
	t.Helper()
	if err := pause(until, systemClock{}); err != nil {
		t.Fatalf("original owned service deadline: %v", err)
	}
}

// configureInputService runs the synthetic native child under a composer catalog; detecting adds
// the synthetic permission-form detector, so serviceInputFormLine opens a detected interaction and
// the run's coverage is tested.
func configureInputService(t *testing.T, fixture *runnerFixture, spec *spawnSpec, until time.Time, detecting bool) {
	t.Helper()
	catalog := syntheticDefinitions()
	permissionRule := jclone(jget(catalog, "codex", "profile", "rules", 0))
	jset(catalog, fixtureAt(syntheticVersion), "codex", "screens", "synthetic-service-submission")
	rule := jget(catalog, "codex", "profile", "rules", 0)
	jset(rule, "synthetic-service-composer", "id")
	jset(rule, "composer_empty", "observation")
	jset(rule, "^Synthetic state$", "region", "start_pattern")
	jset(rule, "^State end$", "region", "end_pattern")
	jset(rule, ja(jo("id", "state", "scope", "line", "pattern", "^Synthetic state$")), "matchers")
	jset(rule, jo("operation", nil, "choices", ja()), "extract")
	jset(rule, true, "verified_noninteraction")
	submission := func(prefix string) any {
		return jo("literal_framing", "plain", "plain_literal_limit", nil, "prefix_hex", prefix, "submit_hex", "0d", "fixture_ids", ja("synthetic-service-submission"))
	}
	jset(catalog, jo("detection", nil, "answers", ja(), "composer", jo(
		"rule_ids", ja("synthetic-service-composer"), "fixture_ids", ja(syntheticFixture),
		"submission", jo("prompt", submission(""), "exec", submission("2120")),
	)), "codex", "profile", "capabilities")
	addSyntheticComposerText(catalog, "codex")
	coverage := "unknown"
	if detecting {
		jarray(jget(catalog, "codex", "profile", "rules")).push(permissionRule)
		jset(catalog, jo("rule_ids", ja("synthetic-permission"), "fixture_ids", ja(syntheticFixture)), "codex", "profile", "capabilities", "detection")
		coverage = "tested"
	}
	if _, err := sqlExecute(fixture.conn, "UPDATE member_runs SET native_version='0.0.1-synthetic',coverage=?2 WHERE run_id=?1", fixture.run.runID, coverage); err != nil {
		t.Fatal(err)
	}
	fixture.refresh(t)
	config := filepath.Join(fixture.dir, "service-input-native.json")
	privateFixtureWrite(t, config, jsonBytes(jo(
		"database_url", databaseURLFor(fixture.databaseFile(t)), "run_id", fixture.run.runID,
		"expires_at_mono_ns", serviceInputExpiration(t, until.Add(realTimeBound)),
	)))
	native := childTestCommand(TestRuntimeService_ControlService_ServiceChild)
	spec.argv = native.Args
	spec.environment = append(spec.environment,
		[2]string{childTestEnv, testName(TestRuntimeService_ControlService_ServiceChild)},
		[2]string{serviceInputNativeEnv, config})
	privateFixtureWrite(t, filepath.Join(fixture.dir, "observation-services.json"), jsonBytes(jo("catalog", catalog)))
}

// servicePeer is a nonblocking control connection read through the production decoder.
type servicePeer struct {
	fd      int
	decoder *controlDecoder
	closed  bool
}

// connectServicePeer connects a peer the test closes once, at close or at cleanup.
func connectServicePeer(t *testing.T, f *serviceFixture) *servicePeer {
	t.Helper()
	syscall.ForkLock.RLock()
	fd, err := unix.Socket(unix.AF_UNIX, unix.SOCK_STREAM, 0)
	if err == nil {
		unix.CloseOnExec(fd)
	}
	syscall.ForkLock.RUnlock()
	if err != nil {
		t.Fatal(err)
	}
	peer := &servicePeer{fd: fd, decoder: newControlDecoder()}
	t.Cleanup(func() { peer.close(t) })
	if err := unix.Connect(fd, &unix.SockaddrUnix{Name: f.artifacts.paths.controlSocket}); err != nil {
		t.Fatal(err)
	}
	if err := unix.SetNonblock(fd, true); err != nil {
		t.Fatal(err)
	}
	return peer
}

func (p *servicePeer) close(t *testing.T) {
	if p.closed {
		return
	}
	p.closed = true
	if err := unix.Close(p.fd); err != nil {
		t.Errorf("closing the control peer: %v", err)
	}
}

func (p *servicePeer) send(t *testing.T, data []byte, until time.Time) {
	t.Helper()
	if err := writeFixtureBytes(p.fd, data, until); err != nil {
		t.Fatal(err)
	}
}

func (p *servicePeer) reply(t *testing.T, until time.Time) jsontext.Value {
	t.Helper()
	buffer := make([]byte, 4096)
	for {
		if !time.Now().Before(until) {
			t.Fatal("original reply observation bound")
		}
		count, err := unix.Read(p.fd, buffer)
		switch {
		case err == nil && count == 0:
			if err := p.decoder.finish(); err != nil {
				t.Fatal(err)
			}
			t.Fatal("owned control disconnected before a complete reply")
		case err == nil:
			values, err := feedValues(p.decoder, buffer[:count])
			if err != nil {
				t.Fatal(err)
			}
			if len(values) > 0 {
				if len(values) != 1 {
					t.Fatalf("one reply, got %d", len(values))
				}
				if err := p.decoder.finish(); err != nil {
					t.Fatal(err)
				}
				return values[0]
			}
		case retryable(err):
			serviceInputPause(t, until)
		default:
			t.Fatalf("owned control read: %v", err)
		}
	}
}

func serviceInputRequest(t *testing.T, f *serviceFixture, key, literal string, kind inputKind, until time.Time) jsontext.Value {
	t.Helper()
	operation := "submit_prompt"
	if kind == inputKindExec {
		operation = "submit_exec"
	}
	return mustJSON(t, jsonObject{
		{"protocol_version", 1}, {"kind", operation}, {"identity", f.preparedIdentity()},
		{"expires_at_mono_ns", serviceInputExpiration(t, until)}, {"operation_key", key},
		{"input_kind", kind}, {"literal", literal},
	})
}

// serviceSubmission requires an accepted, submitted submission reply and decodes it.
func serviceSubmission(t *testing.T, reply jsontext.Value) submissionResult {
	t.Helper()
	result := decodeServiceSubmission(t, reply)
	if result.Status != submissionStatusAccepted || result.Stage != submissionStageSubmitted {
		t.Fatalf("an accepted submission: %s", reply)
	}
	return result
}

func decodeServiceSubmission(t *testing.T, reply jsontext.Value) submissionResult {
	t.Helper()
	if jsonHas(t, reply, "error") {
		t.Fatalf("actual service submission: %s", reply)
	}
	if !jsonIsString(jsonAt(t, reply, "result", "kind"), "submission") {
		t.Fatalf("a submission result: %s", reply)
	}
	var result submissionResult
	if err := decodeStrict(jsonAt(t, reply, "result", "submission"), &result); err != nil {
		t.Fatal(err)
	}
	return result
}

func serviceExchange(t *testing.T, f *serviceFixture, value jsontext.Value, until time.Time) jsontext.Value {
	t.Helper()
	peer := connectServicePeer(t, f)
	defer peer.close(t)
	frame, err := encodeControlFrame(value)
	if err != nil {
		t.Fatal(err)
	}
	peer.send(t, frame, until)
	return peer.reply(t, until)
}

func serviceManual(t *testing.T, f *serviceFixture, line string, until time.Time) {
	t.Helper()
	if err := writeFixtureBytes(f.masterIO, []byte(line), until); err != nil {
		t.Fatal(err)
	}
}

func inputServiceRecords(t *testing.T, f *serviceFixture) []jsontext.Value {
	t.Helper()
	data, err := os.ReadFile(f.path("service-input-lines.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	if len(data) > 65536 {
		t.Fatalf("the native journal holds %d bytes", len(data))
	}
	var records []jsontext.Value
	for line := range bytes.Lines(data) {
		if bytes.HasSuffix(line, []byte("\n")) {
			records = append(records, jsontext.Value(bytes.TrimSuffix(line, []byte("\n"))))
		}
	}
	return records
}

func recordLines(t *testing.T, f *serviceFixture) []string {
	t.Helper()
	var lines []string
	for _, record := range inputServiceRecords(t, f) {
		line, _ := jsonString(jsonIndex(record, "line"))
		lines = append(lines, line)
	}
	return lines
}

func waitInputServiceRecord(t *testing.T, f *serviceFixture, line string, until time.Time) {
	t.Helper()
	for !slices.Contains(recordLines(t, f), line) {
		if _, exited, _ := f.child.tryWait(); exited {
			t.Fatalf("runner remains live while awaiting its native journal: %s", f.connectionDiagnostic("native journal wait"))
		}
		serviceInputPause(t, until)
	}
}

// inputServiceComposer waits for a health reply that observes a non-degraded empty composer.
func inputServiceComposer(t *testing.T, f *serviceFixture, until time.Time) jsontext.Value {
	t.Helper()
	last := jsontext.Value("null")
	for {
		if !time.Now().Before(until) {
			t.Fatalf("the composer verifies empty within the original bound; last health %s; %s", last, f.connectionDiagnostic("composer timeout"))
		}
		health := f.health()
		last = health
		observation := jsonIndex(health, "observation")
		if jsonIsString(jsonAt(t, observation, "eligibility", "state"), "composer_empty") && jsonEqual(jsonIndex(observation, "degraded"), jsontext.Value("false")) {
			return health
		}
		serviceInputPause(t, until)
	}
}

func outputSeqOf(t *testing.T, health jsontext.Value) uint64 {
	t.Helper()
	seq, ok := jsonUint64(jsonIndex(health, "output_seq"))
	if !ok {
		t.Fatalf("health reports its output sequence: %s", health)
	}
	return seq
}

func inputServiceBarrier(t *testing.T, f *serviceFixture, until time.Time) {
	t.Helper()
	before := outputSeqOf(t, f.health())
	serviceManual(t, f, "barrier\r", until)
	waitInputServiceRecord(t, f, "barrier", until)
	for outputSeqOf(t, inputServiceComposer(t, f, until)) <= before {
		serviceInputPause(t, until)
	}
}

// inputServiceSettled waits until the runner observes a non-degraded screen with no open
// interaction and no pending observation work, which also holds on a run without composer
// verification.
func inputServiceSettled(t *testing.T, f *serviceFixture, until time.Time) {
	t.Helper()
	last := jsontext.Value("null")
	for {
		if !time.Now().Before(until) {
			t.Fatalf("the runner settles within the original bound; last health %s; %s", last, f.connectionDiagnostic("settle timeout"))
		}
		last = f.health()
		observation := jsonIndex(last, "observation")
		eligibility := jsonIndex(observation, "eligibility")
		interaction := jsonIsString(jsonIndex(eligibility, "state"), "hold") && jsonIsString(jsonIndex(eligibility, "reason"), "interaction")
		if jsonIsObject(observation) && jsonEqual(jsonIndex(observation, "degraded"), jsontext.Value("false")) && !interaction &&
			jsonEqual(jsonIndex(observation, "pending_count"), jsontext.Value("0")) {
			return
		}
		serviceInputPause(t, until)
	}
}

type inputServiceOptions struct {
	lifetime  time.Duration
	detecting bool
	exitCode  int
	configure func(config any)
	ready     func(t *testing.T, f *serviceFixture, until time.Time)
}

func withInputService(t *testing.T, body func(*serviceFixture, time.Time)) {
	t.Helper()
	runInputService(t, inputServiceOptions{}, body)
}

func withInputServiceCatalog(t *testing.T, configureCatalog func(catalog any), body func(*serviceFixture, time.Time)) {
	t.Helper()
	runInputService(t, inputServiceOptions{configure: func(config any) { configureCatalog(jget(config, "catalog")) }}, body)
}

func withDetectingInputService(t *testing.T, body func(*serviceFixture, time.Time)) {
	t.Helper()
	runInputService(t, inputServiceOptions{detecting: true}, body)
}

// withDetectingInputServiceExiting runs a detecting input service whose body ends the run, which
// then exits with exitCode.
func withDetectingInputServiceExiting(t *testing.T, exitCode int, body func(*serviceFixture, time.Time)) {
	t.Helper()
	runInputService(t, inputServiceOptions{detecting: true, exitCode: exitCode}, body)
}

// withSweepingInputServiceCatalog is a sweeping input service whose catalog configureCatalog
// adjusts first.
func withSweepingInputServiceCatalog(t *testing.T, lifetime time.Duration, detecting bool, configureCatalog func(catalog any), body func(*serviceFixture, time.Time)) {
	t.Helper()
	runInputService(t, inputServiceOptions{
		lifetime: lifetime, detecting: detecting, ready: inputServiceSettled,
		configure: func(config any) {
			jset(config, true, "preview_sweep")
			configureCatalog(jget(config, "catalog"))
		},
	}, body)
}

func runInputService(t *testing.T, options inputServiceOptions, body func(*serviceFixture, time.Time)) {
	t.Helper()
	lifetime := options.lifetime
	if lifetime == 0 {
		lifetime = serviceInputLifetime
	}
	ready := options.ready
	if ready == nil {
		ready = func(t *testing.T, f *serviceFixture, until time.Time) { inputServiceComposer(t, f, until) }
	}
	until := time.Now().Add(lifetime)
	f := launchServiceFixture(t, false, func(t *testing.T, fixture *runnerFixture, spec *spawnSpec) {
		configureInputService(t, fixture, spec, until, options.detecting)
		if options.configure == nil {
			return
		}
		path := filepath.Join(fixture.dir, "observation-services.json")
		data, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		config := parseTestJSON(string(data))
		options.configure(config)
		if err := os.WriteFile(path, jsonBytes(config), 0o600); err != nil {
			t.Fatal(err)
		}
	}, false, some(until))
	defer func() {
		primaryFailed := t.Failed()
		if f.stopped {
			f.resume()
		}
		_ = writeFixtureBytes(f.masterIO, []byte("finish\r"), until)
		status, exited, err := f.child.pollUntil(until)
		if !exited {
			f.child.terminateUntil(until)
		}
		if primaryFailed {
			return
		}
		if err != nil || !exited {
			t.Fatalf("owned service reaped under original deadline: %v", err)
		}
		if code, ok := exitCode(status); !ok || code != options.exitCode {
			t.Fatalf("owned service cleanup retains its original exit expectation %d, got %v: %s", options.exitCode, status, f.connectionDiagnostic("service cleanup"))
		}
		run := mustRun(t, f.fixture.conn, f.fixture.run.runID)
		if run.status != runStatusExited || run.cleanupState != cleanupStateComplete {
			t.Fatalf("run %s cleanup %s: %s", run.status.asStr(), run.cleanupState.asStr(), run.cleanupDiagnosticsJSON)
		}
	}()
	f.startRunning()
	ready(t, f, until)
	body(f, until)
}

// receiptInputs is the run's input receipts, retried while the runner holds the database.
func receiptInputs(conn *dbConn, runID int64, expires uint64) ([]any, error) {
	for {
		sample, err := systemClock{}.monotonicSample()
		if err != nil {
			return nil, err
		}
		if sample.monotonicNs >= expires {
			return nil, appError("original owned native lifetime")
		}
		rows, err := sqlRows(conn, "SELECT input_id,state FROM member_input_operations WHERE run_id=?1 ORDER BY input_id", runID)
		if err == nil {
			inputs := []any{}
			for _, row := range rows {
				inputs = append(inputs, jo("input_id", row[0], "state", row[1]))
			}
			return inputs, nil
		}
		if failure := cafleetErrorOf(err); failure.kind != errorDatabaseBusy {
			return nil, err
		}
		pause := 2 * time.Millisecond
		if remaining := expires - sample.monotonicNs; remaining < uint64(pause) {
			pause = time.Duration(remaining) //nolint:gosec // below two milliseconds
		}
		time.Sleep(pause)
	}
}

// serviceComposerEcho is the synthetic composer painting the unsubmitted line, or nothing while
// no printable text is pending; paste markers and a leading Escape are not text.
func serviceComposerEcho(pending []byte) ([]byte, bool) {
	if bytes.Contains(pending, []byte("\n")) {
		return nil, false
	}
	text := slices.Clone(pending)
	for _, marker := range [][]byte{[]byte("\x1b[200~"), []byte("\x1b[201~")} {
		text = bytes.ReplaceAll(text, marker, nil)
	}
	text = bytes.TrimLeft(text, "\x1b")
	printable := slices.DeleteFunc(text, func(b byte) bool { return b < 0x20 || b == 0x7f })
	if len(printable) == 0 {
		return nil, false
	}
	return syntheticComposerEcho(lossyUTF8(printable)), true
}

// runServiceInputNative is the synthetic native child of an input service: it journals each line
// with the run's receipts at the moment the line arrives, and paints the composer, a permission
// form or a draft as the line selects.
func runServiceInputNative(path string) {
	fail := func(stage string, err error) {
		fmt.Fprintf(os.Stderr, "owned synthetic native %s: %v\n", stage, err)
		os.Exit(1)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		fail("configuration", err)
	}
	config := parseTestJSON(string(data))
	directory := filepath.Dir(path)
	expires, _ := ju64(jget(config, "expires_at_mono_ns"))
	runIDText, _ := jget(config, "run_id").(jsonNumber)
	runID, err := strconv.ParseInt(string(runIDText), 10, 64)
	if err != nil {
		fail("configuration", err)
	}
	databaseURL, _ := jstring(jget(config, "database_url"))
	conn, err := connect(databaseURL)
	if err != nil {
		fail("database", err)
	}
	if err := conn.executeBatch("PRAGMA busy_timeout=0"); err != nil {
		fail("busy timeout", err)
	}
	terminal, err := getTermios(0)
	if err != nil {
		fail("termios", err)
	}
	terminal.Lflag &^= unix.ECHO | unix.ICANON
	if err := setTermios(0, terminal); err != nil {
		fail("termios", err)
	}
	if err := unix.SetNonblock(0, true); err != nil {
		fail("stdin", err)
	}
	journal, err := os.OpenFile(filepath.Join(directory, "service-input-lines.jsonl"), os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		fail("journal", err)
	}
	identity, err := processIdentityCurrent()
	if err != nil {
		fail("identity", err)
	}
	writeIdentity, err := os.OpenFile(filepath.Join(directory, "service-input-native-identity.json"), os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		fail("identity record", err)
	}
	if _, err := writeIdentity.Write(jsonBytes(jo("pid", identity.pid, "start_identity", identity.startIdentity))); err != nil {
		fail("identity record", err)
	}
	_ = writeIdentity.Close()
	write := func(text []byte) {
		if _, err := os.Stdout.Write(text); err != nil {
			fail("output", err)
		}
	}
	write([]byte("review-native-ready\r\n"))
	write([]byte(serviceInputScreen))
	var pending []byte
	formMidWriteShown, composerShown := false, true
	buffer := make([]byte, 4096)
	for {
		sample, err := systemClock{}.monotonicSample()
		if err != nil {
			fail("clock", err)
		}
		if sample.monotonicNs >= expires {
			fail("lifetime", fmt.Errorf("monotonic %d reached the expiry %d", sample.monotonicNs, expires))
		}
		count, err := unix.Read(0, buffer)
		received := false
		switch {
		case err == nil && count == 0:
			return
		case err == nil:
			pending = append(pending, buffer[:count]...)
			received = true
		case retryable(err):
		default:
			fail("input", err)
		}
		if len(pending) > 65536 {
			fail("input", errors.New("pending input exceeds 64 KiB"))
		}
		if received && composerShown {
			if echo, ok := serviceComposerEcho(pending); ok {
				write(echo)
			}
		}
		if !formMidWriteShown && !bytes.Contains(pending, []byte("\n")) && bytes.Contains(pending, []byte(serviceInputFormMidWrite)) {
			formMidWriteShown, composerShown = true, false
			write([]byte(serviceInputForm))
		}
		for {
			end := bytes.IndexByte(pending, '\n')
			if end < 0 {
				break
			}
			line := string(bytes.TrimSuffix(pending[:end], []byte("\r")))
			pending = pending[end+1:]
			switch {
			case strings.HasSuffix(line, "finish"):
				return
			case line == serviceInputFormLine:
				composerShown = false
				write([]byte(serviceInputForm))
				continue
			case line == serviceInputDraftLine:
				composerShown = false
				write([]byte(serviceInputDraft))
				continue
			}
			inputs, err := receiptInputs(conn, runID, expires)
			if err != nil {
				fail("receipts", err)
			}
			if _, err := journal.Write(append(jsonBytes(jo("line", line, "inputs", ja(inputs...))), '\n')); err != nil {
				fail("journal", err)
			}
			if err := journal.Sync(); err != nil {
				fail("journal", err)
			}
			write([]byte("review-answer=" + line + "\r\n"))
			write([]byte(serviceInputScreen))
			composerShown = true
		}
		time.Sleep(2 * time.Millisecond)
	}
}

func shutdownReply(t *testing.T, f *serviceFixture) shutdownOutcome {
	t.Helper()
	reply, err := f.connect("observation fault shutdown").exchange(t, f.request("shutdown", f.preparedIdentity()))
	outcome, decodeErr := shutdownOutcomeDecode(replyResult(t, reply, err))
	if decodeErr != nil {
		t.Fatal(decodeErr)
	}
	return outcome
}

func TestRuntimeService_InputService_ServiceInputSocketSubmissionPersistsTypedOutcomeAndMatchingKeyNeverReplays(t *testing.T) {
	t.Parallel()
	withInputService(t, func(f *serviceFixture, until time.Time) {
		const key, literal = "prompt:service:single", "socket prompt λ"
		value := serviceInputRequest(t, f, key, literal, inputKindPrompt, earliestTime(until, time.Now().Add(realTimeBound)))
		result := serviceSubmission(t, serviceExchange(t, f, value, until))
		id, ok := result.InputID.get()
		if !ok {
			t.Fatal("actual durable service input ID")
		}
		row := storedInput(t, f.fixture.conn, id)
		if row.state != inputStateAccepted || row.operationKey != key {
			t.Fatalf("row %+v", row)
		}
		waitInputServiceRecord(t, f, literal, until)
		if replay := serviceSubmission(t, serviceExchange(t, f, value, until)); replay.InputID != some(id) {
			t.Fatalf("the replay names the original input: %+v", replay)
		}
		inputServiceBarrier(t, f, until)
		lines := recordLines(t, f)
		if count := len(slices.DeleteFunc(slices.Clone(lines), func(line string) bool { return line != literal })); count != 1 {
			t.Fatalf("the literal reaches the native child once: %q", lines)
		}
		if got := storedInput(t, f.fixture.conn, id); got != row {
			t.Fatalf("the replay leaves the row %+v", got)
		}
		for _, record := range inputServiceRecords(t, f) {
			if !jsonIsString(jsonIndex(record, "line"), literal) {
				continue
			}
			inputs, _ := jsonArray(jsonIndex(record, "inputs"))
			if !slices.ContainsFunc(inputs, func(input jsontext.Value) bool {
				recorded, _ := jsonInt64(jsonIndex(input, "input_id"))
				return recorded == id
			}) {
				t.Fatalf("the receipt precedes the native line: %s", record)
			}
		}
	})
}

func earliestTime(a, b time.Time) time.Time {
	if a.Before(b) {
		return a
	}
	return b
}

func errorMentions(reply jsontext.Value, fragments ...string) bool {
	message, ok := jsonString(jsonIndex(reply, "error"))
	return ok && slices.ContainsFunc(fragments, func(fragment string) bool { return strings.Contains(message, fragment) })
}

func TestRuntimeService_InputService_ServiceInputQueuedExpirationKeepsManualProgressAndUsesFreshOutputForExec(t *testing.T) {
	t.Parallel()
	withInputService(t, func(f *serviceFixture, until time.Time) {
		before := outputSeqOf(t, inputServiceComposer(t, f, until))
		expires := time.Now().Add(800 * time.Millisecond)
		value := serviceInputRequest(t, f, "exec:service:expired", "expired command", inputKindExec, expires)
		frame, err := encodeControlFrame(value)
		if err != nil {
			t.Fatal(err)
		}
		queued := connectServicePeer(t, f)
		queued.send(t, frame[:4], until)
		serviceManual(t, f, "manual progress\r", until)
		waitInputServiceRecord(t, f, "manual progress", earliestTime(expires, until))
		var observed uint64
		for {
			observed = outputSeqOf(t, inputServiceComposer(t, f, earliestTime(expires, until)))
			if observed > before {
				break
			}
			serviceInputPause(t, earliestTime(expires, until))
		}
		for !time.Now().After(expires) {
			serviceInputPause(t, until)
		}
		queued.send(t, frame[4:], until)
		if rejected := queued.reply(t, until); !errorMentions(rejected, "deadline") {
			t.Fatalf("the expired request is refused: %s", rejected)
		}
		if repeated := serviceExchange(t, f, value, until); !errorMentions(repeated, "deadline") {
			t.Fatalf("the repeated expired request is refused: %s", repeated)
		}
		if found, err := findInput(context.Background(), f.fixture.conn, f.fixture.run.runID, "exec:service:expired"); err != nil || found.set {
			t.Fatalf("the expired request reserves nothing: %v", err)
		}
		fresh := serviceInputRequest(t, f, "exec:service:fresh", "queued command λ", inputKindExec, earliestTime(until, time.Now().Add(realTimeBound)))
		reply := serviceSubmission(t, serviceExchange(t, f, fresh, until))
		if seq, ok := reply.OutputSeq.get(); !ok || seq < observed {
			t.Fatalf("the exec uses fresh output: %v < %d", reply.OutputSeq, observed)
		}
		waitInputServiceRecord(t, f, "! queued command λ", until)
		inputServiceBarrier(t, f, until)
		if lines := recordLines(t, f); !slices.Equal(lines, []string{"manual progress", "! queued command λ", "barrier"}) {
			t.Fatalf("native lines %q", lines)
		}
	})
}

func TestRuntimeService_InputService_ServiceInputExpiredUnreservedAttemptReleasesPendingWorkForAFreshKey(t *testing.T) {
	t.Parallel()
	const expirationTolerance = 100 * time.Millisecond
	withInputService(t, func(f *serviceFixture, until time.Time) {
		inputServiceComposer(t, f, until)
		if journal := queryText(t, f.fixture.conn, "PRAGMA journal_mode=WAL"); journal != "wal" {
			t.Fatalf("journal mode %s", journal)
		}
		blocker, err := connect(databaseURLFor(f.fixture.databaseFile(t)))
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { closeConnection(t, blocker) })
		mustExecBatch(t, blocker, "PRAGMA busy_timeout=0")
		mustExecBatch(t, blocker, "BEGIN IMMEDIATE")
		const expiredKey = "prompt:service:reservation-expired"
		expires := time.Now().Add(400 * time.Millisecond)
		if !expires.Before(until) {
			t.Fatal("the expiry precedes the service lifetime")
		}
		expired := serviceInputRequest(t, f, expiredKey, "expired reservation must stay unwritten", inputKindPrompt, expires)
		started := time.Now()
		failure := serviceExchange(t, f, expired, until)
		returned := time.Now()
		if blocker.isAutocommit() {
			t.Fatal("the blocker still holds its transaction")
		}
		if returned.Before(expires.Add(-expirationTolerance)) || returned.After(expires.Add(expirationTolerance)) {
			t.Fatalf("original expiration bounds the SQL wait with conservative conversion and scheduling tolerance: elapsed=%v; returned=%v; expires=%v", returned.Sub(started), returned, expires)
		}
		if !errorMentions(failure, "deadline", "database is locked") {
			t.Fatalf("the blocked reservation fails: %s", failure)
		}
		if found, err := findInput(context.Background(), f.fixture.conn, f.fixture.run.runID, expiredKey); err != nil || found.set {
			t.Fatalf("the expired attempt reserves nothing: %v", err)
		}
		if len(inputServiceRecords(t, f)) != 0 {
			t.Fatal("the expired attempt writes nothing to the native child")
		}
		mustExecBatch(t, blocker, "ROLLBACK")
		inputServiceComposer(t, f, until)
		const freshKey, literal = "prompt:service:after-expired-reservation", "fresh after expired reservation"
		fresh := serviceInputRequest(t, f, freshKey, literal, inputKindPrompt, earliestTime(until, time.Now().Add(realTimeBound)))
		accepted := serviceSubmission(t, serviceExchange(t, f, fresh, until))
		id, _ := accepted.InputID.get()
		if row := storedInput(t, f.fixture.conn, id); row.operationKey != freshKey || row.state != inputStateAccepted {
			t.Fatalf("row %+v", row)
		}
		waitInputServiceRecord(t, f, literal, until)
		inputServiceBarrier(t, f, until)
		if lines := recordLines(t, f); !slices.Equal(lines, []string{literal, "barrier"}) {
			t.Fatalf("native lines %q", lines)
		}
		if found, err := findInput(context.Background(), f.fixture.conn, f.fixture.run.runID, expiredKey); err != nil || found.set {
			t.Fatalf("the expired key stays unreserved: %v", err)
		}
	})
}

func TestRuntimeService_InputService_ServiceInputPostWriteFailurePreservesKnownIdentityAndOriginalHistory(t *testing.T) {
	t.Parallel()
	withInputService(t, func(f *serviceFixture, until time.Time) {
		const key, literal = "prompt:service:known-input-error", "SERVICE_R2_PRIVATE_LITERAL_λ"
		mustExecBatch(t, f.fixture.conn, fmt.Sprintf(`CREATE TRIGGER owned_input_accept_failure BEFORE UPDATE OF state ON member_input_operations
			WHEN OLD.operation_key='%s' AND OLD.state='in_progress' AND NEW.state='accepted'
			BEGIN SELECT RAISE(FAIL,'owned accepted update failure'); END`, key))
		value := serviceInputRequest(t, f, key, literal, inputKindPrompt, earliestTime(until, time.Now().Add(realTimeBound)))
		reply := func() jsontext.Value {
			defer mustExecBatch(t, f.fixture.conn, "DROP TRIGGER owned_input_accept_failure")
			return serviceExchange(t, f, value, until)
		}()
		cause, ok := jsonString(jsonIndex(reply, "error"))
		if !ok {
			t.Fatalf("actual Accepted-persistence failure envelope: %s", reply)
		}
		if !strings.Contains(cause, "owned accepted update failure") {
			t.Fatalf("the cause names the failure: %s", cause)
		}
		if strings.Contains(cause, literal) {
			t.Fatal("safe identities replace literal payloads in errors")
		}
		found, err := findInput(context.Background(), f.fixture.conn, f.fixture.run.runID, key)
		if err != nil {
			t.Fatal(err)
		}
		original, ok := found.get()
		if !ok {
			t.Fatal("independent durable reservation after the actual write")
		}
		if original.runID != f.fixture.run.runID || original.operationKey != key || original.state != inputStateUncertain ||
			original.stopCause != some(stopCauseWriteFailed) {
			t.Fatalf("original %+v", original)
		}
		waitInputServiceRecord(t, f, literal, until)
		history := serviceExchange(t, f, value, until)
		retained := decodeServiceSubmission(t, history)
		if retained.InputID != some(original.inputID) || retained.Status != submissionStatusUncertain || retained.Stage != submissionStageSubmitted {
			t.Fatalf("retained %+v", retained)
		}
		inputServiceBarrier(t, f, until)
		if count := len(slices.DeleteFunc(recordLines(t, f), func(line string) bool { return line != literal })); count != 1 {
			t.Fatalf("the literal reaches the native child once, %d times", count)
		}
		if got := storedInput(t, f.fixture.conn, original.inputID); got != original {
			t.Fatalf("the history stays %+v, got %+v", original, got)
		}
		for _, identity := range []string{
			fmt.Sprintf("input_id=%d", original.inputID),
			fmt.Sprintf("run_id=%d", original.runID),
			"operation_key=" + jsonQuoted(t, original.operationKey),
		} {
			if !strings.Contains(cause, identity) {
				t.Fatalf("actual bookkeeping failure retains the independently observed safe input identities: %s lacks %s", cause, identity)
			}
		}
	})
}

func queueBehindTheForm(t *testing.T, f *serviceFixture, until time.Time) int64 {
	t.Helper()
	run := mustRun(t, f.fixture.conn, f.fixture.run.runID)
	serviceManual(t, f, serviceInputFormLine+"\r", until)
	for {
		if !time.Now().Before(until) {
			t.Fatal("the detected interaction opens")
		}
		health, err := nativeInputHost{}.health(run, until)
		if err != nil {
			t.Fatal(err)
		}
		if observation, ok := health.Observation.get(); ok && observation.Eligibility == inputEligibility(inputEligibilityHold{reason: gateReasonInteraction}) && observation.PendingCount == 0 {
			break
		}
		time.Sleep(2 * time.Millisecond)
	}
	value := serviceInputRequest(t, f, "prompt:service:stopped", "queued at shutdown", inputKindPrompt, earliestTime(until, time.Now().Add(realTimeBound)))
	result := decodeServiceSubmission(t, serviceExchange(t, f, value, until))
	if result.Status != submissionStatusQueued || result.Stage != submissionStageZeroBytes {
		t.Fatalf("the prompt queues behind the form: %+v", result)
	}
	id, ok := result.InputID.get()
	if !ok {
		t.Fatal("queued work has a durable input ID")
	}
	return id
}

func assertStoppedResultBeforeExit(t *testing.T, f *serviceFixture, inputID int64) {
	t.Helper()
	rows := tableRows(t, f.fixture.conn, "SELECT kind,json_extract(payload_json,'$.delivery.reason') FROM member_runtime_events WHERE run_id=?1 AND (kind='runner_exited' OR (kind='input_delivery_result' AND json_extract(payload_json,'$.input_id')=?2)) ORDER BY event_seq", f.fixture.run.runID, inputID)
	if len(rows) != 2 || rows[0][0] != "input_delivery_result" || rows[0][1] != "runner_stopped" || rows[1][0] != "runner_exited" || rows[1][1] != nil {
		t.Fatalf("results then exit %v", rows)
	}
	input := storedInput(t, f.fixture.conn, inputID)
	if input.disposition != some(inputDispositionNotSubmitted) || input.deliveryReason != some("runner_stopped") || input.stopCause != none[stopCause]() {
		t.Fatalf("input %+v", input)
	}
	cause := tableRows(t, f.fixture.conn, "SELECT json_extract(payload_json,'$.cause') FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_result' AND json_extract(payload_json,'$.input_id')=?2", f.fixture.run.runID, inputID)
	if len(cause) != 1 || cause[0][0] != nil {
		t.Fatalf("a not-submitted result publishes no uncertain cause: %v", cause)
	}
	run := mustRun(t, f.fixture.conn, f.fixture.run.runID)
	if run.status != runStatusExited || run.cleanupState != cleanupStateComplete {
		t.Fatalf("run %s cleanup %s: %s", run.status.asStr(), run.cleanupState.asStr(), run.cleanupDiagnosticsJSON)
	}
}

func TestRuntimeService_InputService_ServiceNativeExitPublishesQueuedResultsAsRunnerStoppedBeforeRunnerExited(t *testing.T) {
	t.Parallel()
	withDetectingInputService(t, func(f *serviceFixture, until time.Time) {
		id := queueBehindTheForm(t, f, until)
		serviceManual(t, f, "finish\r", until)
		f.assertExitCode(0)
		assertStoppedResultBeforeExit(t, f, id)
	})
}

func TestRuntimeService_InputService_ServiceShutdownPublishesQueuedResultsAsRunnerStoppedBeforeRunnerExited(t *testing.T) {
	t.Parallel()
	withDetectingInputServiceExiting(t, 143, func(f *serviceFixture, until time.Time) {
		id := queueBehindTheForm(t, f, until)
		if outcome := shutdownReply(t, f); outcome.cleanupState != cleanupStateComplete {
			t.Fatalf("shutdown outcome %v: %q", outcome.cleanupState.asStr(), outcome.diagnostics)
		}
		f.assertExitCode(143)
		assertStoppedResultBeforeExit(t, f, id)
	})
}

const injectedFaultCanary = "private-observation-fault-canary"

var observationPhaseNames = map[observationPhase]string{
	observationPhaseChildOutput: "ChildOutput", observationPhaseQuietTimer: "QuietTimer",
	observationPhaseResize: "Resize", observationPhaseShutdownDrain: "ShutdownDrain",
}

var observationOperationNames = map[observationOperation]string{
	observationOperationParser: "Parser", observationOperationDetector: "Detector",
}

// injectedObservationFault fails one observation step, armed by a private file, at its site and
// operation.
type injectedObservationFault struct {
	site      observationPhase
	operation observationOperation
	directory string
}

func (h injectedObservationFault) orderShutdownParsingAfterNativeExit(time.Time) bool { return false }

func (h injectedObservationFault) before(site observationPhase, operation observationOperation) error {
	if site != h.site || operation != h.operation {
		return nil
	}
	armed := filepath.Join(h.directory, "observation-fault-armed")
	if _, err := os.Stat(armed); errors.Is(err, os.ErrNotExist) {
		return nil
	} else if err != nil {
		return ioError(ioErrorText(err))
	}
	if err := os.Remove(armed); err != nil {
		return ioError(ioErrorText(err))
	}
	siteName, siteKnown := observationPhaseNames[site]
	operationName, operationKnown := observationOperationNames[operation]
	if !siteKnown || !operationKnown {
		panic("every observation phase and operation has a fixture name")
	}
	fired, err := os.OpenFile(filepath.Join(h.directory, "observation-fault-fired"), os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return ioError(ioErrorText(err))
	}
	_, err = fired.WriteString(siteName + "/" + operationName)
	if closeErr := fired.Close(); err == nil {
		err = closeErr
	}
	if err != nil {
		return ioError(ioErrorText(err))
	}
	return runtimeError(injectedFaultCanary)
}

// injectedObservationHooks is the observation fault the services configuration selects, if any.
func injectedObservationHooks(value any, directory string) (observationHooks, error) {
	selected := jget(value, "observation_fault")
	if selected == nil {
		return nil, nil //nolint:nilnil // an unselected fault is no hooks
	}
	sites := map[string]observationPhase{
		"child_output": observationPhaseChildOutput, "quiet_timer": observationPhaseQuietTimer,
		"resize": observationPhaseResize, "shutdown_drain": observationPhaseShutdownDrain,
	}
	operations := map[string]observationOperation{"parser": observationOperationParser, "detector": observationOperationDetector}
	siteName, _ := jstring(jget(selected, "site"))
	site, ok := sites[siteName]
	if !ok {
		return nil, runtimeError("owned observation fault site required")
	}
	operationName, _ := jstring(jget(selected, "operation"))
	operation, ok := operations[operationName]
	if !ok {
		return nil, runtimeError("owned observation fault operation required")
	}
	return injectedObservationFault{site: site, operation: operation, directory: directory}, nil
}

func configureObservationFault(t *testing.T, fixture *runnerFixture, spec *spawnSpec, site, operation string) {
	t.Helper()
	syntheticObservationServices(t, fixture, spec)
	path := filepath.Join(fixture.dir, "observation-services.json")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	config := parseTestJSON(string(data))
	jset(config, jo("site", site, "operation", operation), "observation_fault")
	if err := os.WriteFile(path, jsonBytes(config), 0o600); err != nil {
		t.Fatal(err)
	}
}

func armObservationFault(t *testing.T, f *serviceFixture) {
	t.Helper()
	privateFixtureWrite(t, f.path("observation-fault-armed"), []byte("owned one-shot fault"))
}

func assertObservationFaultFired(t *testing.T, f *serviceFixture, expected string) {
	t.Helper()
	path := f.path("observation-fault-fired")
	waitUntilWithin(t, 3*realTimeBound, func() bool { return exists(t, path) })
	fired, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(fired) != expected {
		t.Fatalf("the fault fired at %q, want %q", fired, expected)
	}
	if exists(t, f.path("observation-fault-armed")) {
		t.Fatal("the one-shot fault disarms itself")
	}
}

func assertPrivateCauseStaysLocal(t *testing.T, f *serviceFixture) {
	t.Helper()
	for _, row := range tableRows(t, f.fixture.conn, "SELECT payload_json FROM member_runtime_events UNION ALL SELECT evidence_json FROM member_interactions UNION ALL SELECT text FROM messages UNION ALL SELECT cleanup_diagnostics_json FROM member_runs WHERE cleanup_diagnostics_json IS NOT NULL") {
		if text, _ := row[0].(string); strings.Contains(text, injectedFaultCanary) {
			t.Fatalf("the private fault cause stays local: %s", text)
		}
	}
}

func runtimeEventCount(t *testing.T, f *serviceFixture, kind string) int64 {
	t.Helper()
	return queryInt64(t, f.fixture.conn, "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind=?2", f.fixture.run.runID, kind)
}

func assertDurableGap(t *testing.T, f *serviceFixture) {
	t.Helper()
	waitUntilWithin(t, 3*realTimeBound, func() bool { return runtimeEventCount(t, f, "runner_degraded") >= 2 })
	var causes []jsontext.Value
	for _, row := range tableRows(t, f.fixture.conn, "SELECT payload_json FROM member_runtime_events WHERE run_id=?1 AND kind='runner_degraded' ORDER BY event_seq", f.fixture.run.runID) {
		causes = append(causes, jsonIndex(jsontext.Value(row[0].(string)), "cause"))
	}
	observerError := slices.ContainsFunc(causes, func(cause jsontext.Value) bool {
		gaps, _ := jsonUint64(jsonIndex(cause, "gap_count"))
		return jsonIsString(jsonIndex(cause, "kind"), "observer_error") && gaps > 0
	})
	recovered := slices.ContainsFunc(causes, func(cause jsontext.Value) bool { return jsonIsString(jsonIndex(cause, "kind"), "storage_recovered") })
	if !observerError || !recovered {
		t.Fatalf("an observer-error gap and its recovery are durable: %s", causes)
	}
}

// installTrap makes the synthetic native script run command on TERM.
func installTrap(t *testing.T, spec *spawnSpec, command string) {
	t.Helper()
	script := spec.argv[0]
	body, err := os.ReadFile(script)
	if err != nil {
		t.Fatal(err)
	}
	prefix, suffix, found := strings.Cut(string(body), "while IFS=")
	if !found {
		t.Fatal("owned synthetic input loop")
	}
	if err := os.WriteFile(script, []byte(prefix+"trap '"+command+"' TERM\nwhile IFS="+suffix), 0o700); err != nil {
		t.Fatal(err)
	}
}

// resize resizes the outer terminal and signals the runner the way a terminal emulator does.
func (f *serviceFixture) resize(rows, cols uint16) {
	f.t.Helper()
	if err := pty.Setsize(f.master, &pty.Winsize{Rows: rows, Cols: cols}); err != nil {
		f.t.Fatal(err)
	}
	if err := unix.Kill(f.child.pid, unix.SIGWINCH); err != nil {
		f.t.Fatal(err)
	}
}

func startedChild(t *testing.T, child jsontext.Value) processIdentity {
	t.Helper()
	var identity processIdentity
	if err := decodeStrict(child, &identity); err != nil {
		t.Fatal(err)
	}
	return identity
}

func previewLineWithResume(message messageRecord) string {
	return fmt.Sprintf("[cafleet msg %d from %d %s] %s [cafleet] Resume your work if something was still running.",
		message.messageID, message.fromMemberID, message.createdAt, message.text)
}

func countContaining(lines []string, text string) int {
	count := 0
	for _, line := range lines {
		if strings.Contains(line, text) {
			count++
		}
	}
	return count
}

func waitForCondition(t *testing.T, until time.Time, what string, done func() bool) {
	t.Helper()
	for !done() {
		if !time.Now().Before(until) {
			t.Fatal(what)
		}
		time.Sleep(5 * time.Millisecond)
	}
}

// previewResults is the receipt-free delivery results the run published for messageID.
func previewResults(t *testing.T, f *serviceFixture, messageID int64) []jsontext.Value {
	t.Helper()
	var results []jsontext.Value
	for _, row := range tableRows(t, f.fixture.conn, "SELECT payload_json FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_result' AND json_extract(payload_json,'$.message_id')=?2 AND json_type(payload_json,'$.input_id')='null' ORDER BY event_seq", f.fixture.run.runID, messageID) {
		results = append(results, jsontext.Value(row[0].(string)))
	}
	return results
}

// typeBarrier types barrier and waits for its journal line, which also carries any literal the
// native child still holds unsubmitted; the child then restores the empty composer.
func typeBarrier(t *testing.T, f *serviceFixture, until time.Time) {
	t.Helper()
	serviceManual(t, f, "barrier\r", until)
	waitForCondition(t, until, "the barrier line is journaled", func() bool {
		return slices.ContainsFunc(recordLines(t, f), func(line string) bool { return strings.HasSuffix(line, "barrier") })
	})
}

// clearWithBarrier is typeBarrier followed by the verified empty composer it restores.
func clearWithBarrier(t *testing.T, f *serviceFixture, until time.Time) {
	t.Helper()
	typeBarrier(t, f, until)
	inputServiceSettled(t, f, until)
}

// assertInputHeld checks that the unverified run still holds input: without composer
// verification it never reports a verified empty composer.
func assertInputHeld(t *testing.T, f *serviceFixture) {
	t.Helper()
	health := observationHealthOf(t, f)
	if !jsonIsString(jsonAt(t, health, "observation", "eligibility", "state"), "hold") {
		t.Fatalf("the run holds input: %s", health)
	}
}

func sweepSubmit(t *testing.T, f *serviceFixture, key, literal string, kind inputKind, until time.Time) (submissionResult, jsontext.Value) {
	t.Helper()
	reply := serviceExchange(t, f, serviceInputRequest(t, f, key, literal, kind, until), until)
	return decodeServiceSubmission(t, reply), reply
}

func sweepReconcile(t *testing.T, f *serviceFixture, inputID int64, until time.Time) {
	t.Helper()
	value := mustJSON(t, jsonObject{
		{"protocol_version", 1}, {"kind", "receipt"}, {"identity", f.preparedIdentity()},
		{"expires_at_mono_ns", serviceInputExpiration(t, until)}, {"action", "reconcile"}, {"input_id", inputID},
		{"expected_state", "uncertain"}, {"disposition", "submitted"}, {"evidence_ref", "audit:pane-shows-the-input"},
	})
	replyResult(t, serviceExchange(t, f, value, until), nil)
}

func withoutComposerVerification(catalog any) {
	jset(catalog, nil, "codex", "profile", "capabilities", "composer")
}

func sendToMember(t *testing.T, f *serviceFixture, body string) messageRecord {
	t.Helper()
	message, err := sendMessage(context.Background(), f.fixture.conn, f.fixture.run.directorMemberID, f.fixture.run.memberID, body)
	if err != nil {
		t.Fatal(err)
	}
	return message
}

func notifiedAt(t *testing.T, f *serviceFixture, messageID int64) bool {
	t.Helper()
	return queryInt64(t, f.fixture.conn, "SELECT notified_at IS NOT NULL FROM messages WHERE message_id=?1", messageID) == 1
}

// stoppedPreview sends a preview whose write a detected form stops, and checks that its stamp
// precedes its published result.
func stoppedPreview(t *testing.T, f *serviceFixture, until time.Time) messageRecord {
	t.Helper()
	message := sendToMember(t, f, "unverified preview "+serviceInputFormMidWrite)
	waitForCondition(t, until, "the stopped preview publishes its result", func() bool { return len(previewResults(t, f, message.messageID)) > 0 })
	if !notifiedAt(t, f, message.messageID) {
		t.Fatal("the stamp precedes the published result")
	}
	return message
}

func TestRuntimeService_PreviewSweep_AnUnverifiedRunStampsAnUncertainPreviewBeforePublishingItAndNeverRetypesIt(t *testing.T) {
	t.Parallel()
	// Deliberate divergence from the Rust test, which races candidateSettle against the barrier: an unverified form episode need not close, so no settled composer can precede a retype check.
	withSweepingInputServiceCatalog(t, 120*time.Second, true, withoutComposerVerification, func(f *serviceFixture, until time.Time) {
		message := stoppedPreview(t, f, until)
		typeBarrier(t, f, until)
		time.Sleep(scenarioSweepWindow)
		if len(previewResults(t, f, message.messageID)) != 1 {
			t.Fatal("one published result")
		}
		assertInputHeld(t, f)
	})
}

func TestRuntimeService_PreviewSweep_AVerifiedRunLeavesAnUncertainPreviewUnnotifiedAndRetypesItOnceTheComposerIsVerifiedEmpty(t *testing.T) {
	t.Parallel()
	withSweepingInputServiceCatalog(t, 40*time.Second, true, func(any) {}, func(f *serviceFixture, until time.Time) {
		message := sendToMember(t, f, "verified preview "+serviceInputFormMidWrite)
		waitForCondition(t, until, "the stopped preview publishes its result", func() bool { return len(previewResults(t, f, message.messageID)) > 0 })
		if notifiedAt(t, f, message.messageID) {
			t.Fatal("the uncertain preview stays unnotified")
		}
		clearWithBarrier(t, f, until)
		line := previewLineWithResume(message)
		waitForCondition(t, until, "the sweep retypes the preview whole", func() bool {
			return slices.ContainsFunc(recordLines(t, f), func(typed string) bool {
				return strings.Contains(typed, line) && !strings.HasSuffix(typed, "barrier")
			})
		})
		waitForCondition(t, until, "the retyped preview is stamped", func() bool { return notifiedAt(t, f, message.messageID) })
	})
}

// uncertainPromptHoldsTheFifoAndTheSweep stops a prompt mid-write with a detected form, types the
// barrier (settling the composer when settle is set), then checks that neither a queued exec nor a
// sweep preview writes a byte while the prompt is uncertain.
func uncertainPromptHoldsTheFifoAndTheSweep(t *testing.T, f *serviceFixture, until time.Time, settle bool) (submissionResult, messageRecord) {
	t.Helper()
	stopped, stoppedReply := sweepSubmit(t, f, "prompt:stopped-mid-write", "stopped prompt "+serviceInputFormMidWrite, inputKindPrompt, earliestTime(until, time.Now().Add(realTimeBound)))
	if stopped.Status != submissionStatusUncertain {
		t.Fatalf("the stopped prompt is uncertain: %s", stoppedReply)
	}
	if settle {
		clearWithBarrier(t, f, until)
	} else {
		typeBarrier(t, f, until)
	}
	queued, queuedReply := sweepSubmit(t, f, "exec:behind-the-uncertain-prompt", "printf behind", inputKindExec, earliestTime(until, time.Now().Add(realTimeBound)))
	if queued.Status != submissionStatusQueued {
		t.Fatalf("the exec queues: %s", queuedReply)
	}
	message := sendToMember(t, f, "preview behind the uncertain prompt")
	time.Sleep(scenarioSweepWindow)
	if before := recordLines(t, f); len(before) != 1 {
		t.Fatalf("only the stopped prompt's line: %q", before)
	}
	if notifiedAt(t, f, message.messageID) {
		t.Fatal("the preview waits")
	}
	return stopped, message
}

func TestRuntimeService_PreviewSweep_AnUncertainPromptStopsQueuedInputAndTheSweepUntilReconciledOnAnUnverifiedRun(t *testing.T) {
	t.Parallel()
	// Deliberate divergence from the Rust test, which races candidateSettle against the barrier: an unverified form episode never closes, so the verified sibling proves the drain after reconciliation.
	withSweepingInputServiceCatalog(t, 40*time.Second, true, withoutComposerVerification, func(f *serviceFixture, until time.Time) {
		uncertainPromptHoldsTheFifoAndTheSweep(t, f, until, false)
		assertInputHeld(t, f)
	})
}

func TestRuntimeService_PreviewSweep_AnUncertainPromptStopsQueuedInputAndTheSweepUntilReconciledOnAVerifiedRun(t *testing.T) {
	t.Parallel()
	withSweepingInputServiceCatalog(t, 40*time.Second, true, func(any) {}, func(f *serviceFixture, until time.Time) {
		stopped, message := uncertainPromptHoldsTheFifoAndTheSweep(t, f, until, true)
		id, ok := stopped.InputID.get()
		if !ok {
			t.Fatal("the stopped prompt has a receipt")
		}
		sweepReconcile(t, f, id, until)
		line := previewLineWithResume(message)
		waitForCondition(t, until, "the queued exec and the preview drain", func() bool {
			lines := recordLines(t, f)
			return slices.ContainsFunc(lines, func(typed string) bool { return strings.HasSuffix(typed, "printf behind") }) &&
				slices.ContainsFunc(lines, func(typed string) bool { return strings.Contains(typed, line) })
		})
		lines := recordLines(t, f)
		exec := slices.IndexFunc(lines, func(typed string) bool { return strings.HasSuffix(typed, "printf behind") })
		preview := slices.IndexFunc(lines, func(typed string) bool { return strings.Contains(typed, line) })
		if exec >= preview {
			t.Fatalf("the FIFO drains before the sweep: %q", lines)
		}
	})
}

func observationHealthOf(t *testing.T, f *serviceFixture) jsontext.Value {
	t.Helper()
	return f.health()
}

func TestRuntimeService_ObservationContract_ServiceSqliteAndWriterFailuresKeepHealthManualDisplayAndResizeLive(t *testing.T) {
	t.Parallel()
	for _, boundary := range []string{"fail-persist", "block-persist"} {
		t.Run(boundary, func(t *testing.T) {
			t.Parallel()
			f := configuredServiceFixture(t, false, syntheticObservationServices)
			f.startRunning()
			hook := f.path(boundary)
			privateFixtureWrite(t, hook, []byte("owned test gate"))
			f.writeOuter("form-pressure\r")
			f.readUntil("Operation: write form-pressure.txt")
			waitUntilWithin(t, 3*realTimeBound, func() bool {
				return jsonEqual(jsonAt(t, observationHealthOf(t, f), "observation", "degraded"), jsontext.Value("true")) || exists(t, f.path("persist-entered"))
			})
			mustExecBatch(t, f.fixture.conn, "BEGIN IMMEDIATE")
			begin := time.Now()
			status := observationHealthOf(t, f)
			if elapsed := time.Since(begin); elapsed >= realTimeBound/2 {
				t.Fatalf("%s: health answers while storage is locked, took %v", boundary, elapsed)
			}
			if !jsonIsString(jsonAt(t, status, "observation", "eligibility", "state"), "hold") {
				t.Fatalf("%s: input holds: %s", boundary, status)
			}
			if pending, _ := jsonUint64(jsonAt(t, status, "observation", "pending_count")); pending > 130 {
				t.Fatalf("%s: pending %d is bounded", boundary, pending)
			}
			f.assertManualIO()
			sequence := outputSeqOf(t, observationHealthOf(t, f))
			if process, _ := f.process.get(); !mustMatch(t, process) {
				t.Fatalf("%s: the runner keeps its generation", boundary)
			}
			f.resize(25, 90)
			waitUntilWithin(t, 3*realTimeBound, func() bool { return outputSeqOf(t, observationHealthOf(t, f)) > sequence })
			mustExecBatch(t, f.fixture.conn, "ROLLBACK")
			if err := os.Remove(hook); err != nil {
				t.Fatal(err)
			}
			waitUntilWithin(t, 3*realTimeBound, func() bool { return runtimeEventCount(t, f, "interaction_opened") == 1 })
			f.finish()
		})
	}
}

func TestRuntimeService_ObservationContract_ServiceEachDurableHistoryTableIsCheckedBeforeNativeSpawn(t *testing.T) {
	t.Parallel()
	for _, interaction := range []bool{false, true} {
		t.Run(fmt.Sprintf("interaction=%v", interaction), func(t *testing.T) {
			t.Parallel()
			var seeded string
			f := launchServiceFixture(t, false, func(t *testing.T, fixture *runnerFixture, spec *spawnSpec) {
				syntheticObservationServices(t, fixture, spec)
				run := fixture.run
				evidence, err := runEvidence(run, "runner-lifecycle", 1)
				if err != nil {
					t.Fatal(err)
				}
				if interaction {
					seeded = jsonText(evidence)
					digest, _ := jsonString(jsonIndex(evidence, "evidence_digest"))
					if _, err := sqlExecute(fixture.conn, "INSERT INTO member_interactions(run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json) VALUES(?1,1,'permission_request',?2,'2026-10-26T17:00:00Z',?3)", run.runID, digest, seeded); err != nil {
						t.Fatal(err)
					}
					return
				}
				payload := mustJSON(t, jsonObject{
					{"event_id", 1}, {"event_seq", 1}, {"run_id", run.runID}, {"member_id", run.memberID}, {"fleet_id", run.fleetID},
					{"director_member_id", run.directorMemberID}, {"kind", "coverage_unknown"}, {"interaction_id", nil},
					{"classification", nil}, {"observed_at", "2026-10-26T17:00:00Z"}, {"source", "runner_lifecycle"},
					{"decision_outcome", "unknown"}, {"director_action", nil}, {"evidence", evidence},
					{"cause", jsonObject{{"kind", "coverage_unknown"}}},
				})
				seeded = jsonText(payload)
				if _, err := sqlExecute(fixture.conn, "INSERT INTO member_runtime_events(run_id,event_seq,kind,observed_at,payload_json,notification_state,archive_reason) VALUES(?1,1,'coverage_unknown','2026-10-26T17:00:00Z',?2,'archived','member_inactive')", run.runID, seeded); err != nil {
					t.Fatal(err)
				}
			}, true, none[time.Time]())
			run := mustRun(t, f.fixture.conn, f.fixture.run.runID)
			if _, spawned := run.childPID.get(); spawned || exists(t, f.path("native-started")) {
				t.Fatal("nonfresh history refuses before the native spawn")
			}
			wantEvents, wantInteractions := int64(1), int64(0)
			query := "SELECT payload_json FROM member_runtime_events WHERE run_id=?1"
			if interaction {
				wantEvents, wantInteractions = 0, 1
				query = "SELECT evidence_json FROM member_interactions WHERE run_id=?1"
			}
			if got := runtimeEventCount(t, f, "coverage_unknown"); got != wantEvents {
				t.Fatalf("coverage_unknown events %d, want %d", got, wantEvents)
			}
			if got := queryInt64(t, f.fixture.conn, "SELECT COUNT(*) FROM member_interactions WHERE run_id=?1", run.runID); got != wantInteractions {
				t.Fatalf("interactions %d, want %d", got, wantInteractions)
			}
			if retained := queryText(t, f.fixture.conn, query, run.runID); retained != seeded {
				t.Fatalf("the seeded history is retained verbatim: %s", retained)
			}
		})
	}
}

func TestRuntimeService_InjectedFailures_RunnerFailureWithALiveNativeChildPersistsDegradedBeforeOwnedCleanupConfirmsExit(t *testing.T) {
	t.Parallel()
	f := configuredServiceFixture(t, false, func(t *testing.T, fixture *runnerFixture, spec *spawnSpec) {
		syntheticObservationServices(t, fixture, spec)
		installTrap(t, spec, "")
	})
	child := startedChild(t, f.startRunning())
	if !mustMatch(t, child) {
		t.Fatal("the native child is live")
	}
	if process, _ := f.process.get(); !mustMatch(t, process) {
		t.Fatal("the runner is live")
	}
	before := mustRun(t, f.fixture.conn, f.fixture.run.runID)
	if before.status != runStatusRunning {
		t.Fatalf("run %s", before.status.asStr())
	}
	f.resize(0, 0)
	until := time.Now().Add(realTimeBound)
	var during runRecord
	for {
		during = mustRun(t, f.fixture.conn, f.fixture.run.runID)
		if during.status != runStatusRunning {
			break
		}
		if !time.Now().Before(until) {
			t.Fatal("runner failure left the run running")
		}
		time.Sleep(2 * time.Millisecond)
	}
	if during.status != runStatusDegraded || during.childPID != some(child.pid) {
		t.Fatalf("run %s child %v", during.status.asStr(), during.childPID)
	}
	if !mustMatch(t, child) {
		t.Fatal("the TERM-immune native child is still live while the run is degraded")
	}
	status := f.wait()
	run := mustRun(t, f.fixture.conn, f.fixture.run.runID)
	diagnostics := jsonStrings(t, run.cleanupDiagnosticsJSON)
	if !slices.ContainsFunc(diagnostics, func(text string) bool { return strings.Contains(text, "terminal dimensions must be positive") }) {
		t.Fatalf("diagnostics %q", diagnostics)
	}
	if run.status != runStatusExited || run.childPID != some(child.pid) || run.childStartIdentity != some(child.startIdentity) ||
		run.ownerEpoch != before.ownerEpoch || run.cleanupState != cleanupStateIncomplete {
		t.Fatalf("run %s child %v epoch %d cleanup %s: %q", run.status.asStr(), run.childPID, run.ownerEpoch, run.cleanupState.asStr(), diagnostics)
	}
	if code, ok := exitCode(status); !ok || code != 137 {
		t.Fatalf("the runner exits 137, got %v", status)
	}
	if mustMatch(t, child) {
		t.Fatal("owned cleanup ends the native child")
	}
}

func TestRuntimeService_InjectedFailures_ShutdownDetectorFailureRecordsGapBeforeTruthfulNativeExit(t *testing.T) {
	t.Parallel()
	f := configuredServiceFixture(t, false, func(t *testing.T, fixture *runnerFixture, spec *spawnSpec) {
		configureObservationFault(t, fixture, spec, "shutdown_drain", "detector")
		installTrap(t, spec, `printf "\033[2J\033[19;1HSynthetic permission request\r\nOperation: write shutdown-failure.txt\r\n> Allow once\r\n  Deny once\r\nRequest form: synthetic-only\r\nSelection: "; sleep 0.2; exit 0`)
	})
	f.startRunning()
	armObservationFault(t, f)
	begin := time.Now()
	outcome := shutdownReply(t, f)
	if elapsed := time.Since(begin); elapsed >= 3*time.Second {
		t.Fatalf("the shutdown returns within its schedule, took %v", elapsed)
	}
	assertObservationFaultFired(t, f, "ShutdownDrain/Detector")
	if outcome.nativeExit != some[childExit](childExitCode{code: 0}) || outcome.cleanupState != cleanupStateComplete {
		t.Fatalf("outcome %v %s: %q", outcome.nativeExit, outcome.cleanupState.asStr(), outcome.diagnostics)
	}
	assertDurableGap(t, f)
	f.assertExitCode(0)
	if count := runtimeEventCount(t, f, "runner_exited"); count != 1 {
		t.Fatalf("one runner_exited event, got %d", count)
	}
	assertPrivateCauseStaysLocal(t, f)
}
