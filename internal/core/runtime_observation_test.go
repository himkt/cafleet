package core

import (
	"bytes"
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"math"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"regexp"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"testing"
	"time"
	"unicode"
	"unicode/utf8"

	"github.com/himkt/cafleet/internal/vt"
)

func TestRuntimeObservation_InputEligibilityRendersComposerEmptyOrHoldWithItsReason(t *testing.T) {
	t.Parallel()
	cases := []struct {
		eligibility inputEligibility
		expected    string
	}{
		{inputEligibilityComposerEmpty{}, "composer_empty"},
		{inputEligibilityHold{reason: gateReasonCandidate}, "hold:candidate"},
		{inputEligibilityHold{reason: gateReasonInteraction}, "hold:interaction"},
		{inputEligibilityHold{reason: gateReasonUnknown}, "hold:unknown"},
		{inputEligibilityHold{reason: gateReasonObserverUnavailable}, "hold:observer_unavailable"},
		{inputEligibilityHold{reason: gateReasonGap}, "hold:gap"},
	}
	for _, tc := range cases {
		if got := tc.eligibility.rendering(); got != tc.expected {
			t.Fatalf("%s: rendering = %q", tc.expected, got)
		}
	}
}

const observationFixtureNow = "2026-10-26T16:40:00.000000+00:00"

// observationFixtureClock is a virtual clock whose wall time starts at observationFixtureNow.
func observationFixtureClock(t *testing.T) *testClock {
	t.Helper()
	origin, ok := parseLenient(observationFixtureNow)
	if !ok {
		t.Fatal("the fixture stamp is canonical")
	}
	return testClockStartingAt(time.Now()).withWallOrigin(origin)
}

const observationTrackerNow = "2026-10-26T15:35:00.000000+00:00"

func newTestTracker(t *testing.T) *episodeTracker {
	t.Helper()
	tracker, err := newEpisodeTracker(1)
	if err != nil {
		t.Fatal(err)
	}
	return tracker
}

func mustObserve(t *testing.T, tracker *episodeTracker, value screenObservation, now time.Time) episodeUpdate {
	t.Helper()
	update, err := tracker.observe(&value, now, observationTrackerNow)
	if err != nil {
		t.Fatal(err)
	}
	return update
}

// openEpisode shows value as a candidate at origin and opens its episode 100 ms later.
func openEpisode(t *testing.T, tracker *episodeTracker, value screenObservation, origin time.Time) episodeUpdate {
	t.Helper()
	candidate := mustObserve(t, tracker, value, origin)
	if len(candidate.transitions) != 0 || candidate.eligibility != (inputEligibilityHold{reason: gateReasonCandidate}) {
		t.Fatalf("candidate = %+v", candidate)
	}
	update := mustObserve(t, tracker, value, origin.Add(100*time.Millisecond))
	if len(update.transitions) != 1 || update.transitions[0].kind != runtimeEventKindInteractionOpened {
		t.Fatalf("opening = %+v", update)
	}
	return update
}

func sequenced(value screenObservation, sequence uint64) screenObservation {
	value.outputSeq = sequence
	value.rawEvidence.screenSeq = sequence
	return value
}

func gatedObservation(t *testing.T, data []byte, sequence uint64) screenObservation {
	t.Helper()
	catalog := mustLoadCatalog(t, gatedFixtureDefinition())
	state := mustTerminalState(t, terminalSize{rows: 24, cols: 80})
	mustFeed(t, state, data)
	snapshot := state.snapshot()
	value, err := newDetector(catalog, codingAgentCodex, some(syntheticVersion)).observe(&snapshot)
	if err != nil {
		t.Fatal(err)
	}
	return sequenced(value, sequence)
}

func gatedRequest(t *testing.T, sequence uint64) screenObservation {
	t.Helper()
	return gatedObservation(t, gatedFixtureForm("write disposable.txt", "deny"), sequence)
}

func gatedDialog(t *testing.T, sequence uint64) screenObservation {
	t.Helper()
	value := gatedObservation(t, gatedReasonDialog(none[string]()), sequence)
	if value.kind != observationKindPermissionContinuation {
		t.Fatalf("kind = %v", value.kind)
	}
	return value
}

func TestRuntimeObservation_ContinuationEpisodes_ContinuationKeepsTheActiveEpisodeOpenAndVisibleWithoutEvents(t *testing.T) {
	t.Parallel()
	origin := time.Now()
	tracker := newTestTracker(t)
	openEpisode(t, tracker, gatedRequest(t, 1), origin)
	episode := tracker.activeEpisode()
	if episode == none[episodeRef]() {
		t.Fatal("an episode is active")
	}
	for index, millis := range []int{200, 400, 2300, 4500} {
		update := mustObserve(t, tracker, gatedDialog(t, uint64(2+index)), origin.Add(time.Duration(millis)*time.Millisecond))
		if len(update.transitions) != 0 || update.visibility != visibilityVisible || update.eligibility != (inputEligibilityHold{reason: gateReasonInteraction}) || tracker.activeEpisode() != episode {
			t.Fatalf("a continuation emits no event at %d ms: %+v", millis, update)
		}
	}
	returned := mustObserve(t, tracker, gatedRequest(t, 9), origin.Add(5*time.Second))
	if len(returned.transitions) != 0 || returned.visibility != visibilityVisible || tracker.activeEpisode() != episode {
		t.Fatalf("returned = %+v", returned)
	}
}

func TestRuntimeObservation_ContinuationEpisodes_ContinuationWithoutAnActiveEpisodeIsAnUnknownScreen(t *testing.T) {
	t.Parallel()
	origin := time.Now()
	continuation := newTestTracker(t)
	unknown := newTestTracker(t)
	blank := gatedObservation(t, []byte("\x1b[0m\x1b[2J"), 1)
	for _, millis := range []uint64{0, 150, 400, 2500} {
		at := origin.Add(time.Duration(millis) * time.Millisecond) //nolint:gosec // a fixed millisecond offset below 2500
		left := mustObserve(t, continuation, gatedDialog(t, 1+millis), at)
		right := mustObserve(t, unknown, sequenced(blank, 1+millis), at)
		if len(left.transitions) != 0 || left.visibility != right.visibility || left.eligibility != right.eligibility || continuation.activeEpisode() != none[episodeRef]() {
			t.Fatalf("%d ms: %+v vs %+v", millis, left, right)
		}
	}
}

// parserAlgorithmBridge feeds every parsed screen through the detector and the tracker.
type parserAlgorithmBridge struct {
	t              *testing.T
	detector       *detector
	tracker        *episodeTracker
	now            time.Time
	visits         []bool
	candidateHolds int
	transitions    int
}

func (b *parserAlgorithmBridge) inputHealth() observationHealth {
	panic("the parser bridge is never asked to quiesce input")
}

func (b *parserAlgorithmBridge) parserFailure() error {
	return nil
}

func (b *parserAlgorithmBridge) observe(screen *screenSnapshot) error {
	value, err := b.detector.observe(screen)
	if err != nil {
		return err
	}
	b.visits = append(b.visits, value.kind == observationKindPermissionRequest)
	update, err := b.tracker.observe(&value, b.now, observationTrackerNow)
	if err != nil {
		return err
	}
	if update.eligibility == (inputEligibilityHold{reason: gateReasonCandidate}) {
		b.candidateHolds++
	}
	b.transitions += len(update.transitions)
	return nil
}

func TestRuntimeObservation_RuntimeDetectorParserBridgeVisitsATransientCandidateBeforeALaterClear(t *testing.T) {
	t.Parallel()
	catalog := mustLoadCatalog(t, syntheticDefinitions())
	bridge := &parserAlgorithmBridge{t: t, detector: newDetector(catalog, codingAgentCodex, some(syntheticVersion)), tracker: newTestTracker(t), now: time.Now()}
	state := mustTerminalState(t, terminalSize{rows: 24, cols: 80})
	data := "\x1b[2J\x1b[H" + strings.Join(permissionLines("write disposable.txt"), "\r\n") + "\x1b[2J\x1b[Hordinary working output"
	if err := state.feedObserved([]byte(data), bridge); err != nil {
		t.Fatal(err)
	}
	candidate := slices.Index(bridge.visits, true)
	if candidate < 0 {
		t.Fatal("the transient complete form reaches the algorithms")
	}
	if !slices.Contains(bridge.visits[candidate+1:], false) || bridge.candidateHolds == 0 || bridge.transitions != 0 {
		t.Fatalf("visits %v holds %d transitions %d", bridge.visits, bridge.candidateHolds, bridge.transitions)
	}
}

func unknownObservation(t *testing.T, sequence uint64) screenObservation {
	t.Helper()
	snapshot := syntheticTerminal(t, nil, 80, false).snapshot()
	return sequenced(syntheticObservation(t, syntheticDefinitions(), &snapshot), sequence)
}

func secretObservation(t *testing.T, secret string, cols uint16, sequence uint64) screenObservation {
	t.Helper()
	definition := syntheticDefinitions()
	rule := jget(definition, "codex", "profile", "rules", 0)
	jset(rule, "^> (?P<label>Allow "+regexp.QuoteMeta(secret)+")$", "matchers", 1, "pattern")
	jset(rule, "^  (?P<label>Deny "+regexp.QuoteMeta(secret)+")$", "matchers", 2, "pattern")
	rows := permissionLines(secret)
	rows[2] = "> Allow " + secret
	rows[3] = "  Deny " + secret
	snapshot := syntheticTerminal(t, rows, cols, false).snapshot()
	value := sequenced(syntheticObservation(t, definition, &snapshot), sequence)
	raw := mustJSON(t, value.rawEvidence)
	for _, field := range []string{"operation", "choices", "excerpt"} {
		if !strings.Contains(string(jsonAt(t, raw, field)), secret) {
			t.Fatalf("canary missing from raw %s", field)
		}
	}
	return value
}

type timelineScreenKind uint8

const (
	timelinePermission timelineScreenKind = iota + 1
	timelineComposerEmpty
	timelineUnknown
)

// timelineScreen is a screen an episode timeline shows the tracker; an unset sequence keeps the
// fixture's own.
type timelineScreen struct {
	kind      timelineScreenKind
	operation string
	sequence  opt[uint64]
}

func permissionScreen(operation string, sequence opt[uint64]) timelineScreen {
	return timelineScreen{kind: timelinePermission, operation: operation, sequence: sequence}
}

func composerScreen(sequence opt[uint64]) timelineScreen {
	return timelineScreen{kind: timelineComposerEmpty, sequence: sequence}
}

func unknownScreen(sequence uint64) timelineScreen {
	return timelineScreen{kind: timelineUnknown, sequence: some(sequence)}
}

func (s timelineScreen) observation(t *testing.T) screenObservation {
	t.Helper()
	var value screenObservation
	switch s.kind {
	case timelinePermission:
		value = permissionObservation(t, s.operation)
	case timelineComposerEmpty:
		value = stateObservation(t, "composer_empty", true)
	case timelineUnknown:
		sequence, _ := s.sequence.get()
		return unknownObservation(t, sequence)
	default:
		panic("invalid timelineScreenKind")
	}
	if sequence, set := s.sequence.get(); set {
		return sequenced(value, sequence)
	}
	return value
}

type timelineOpKind uint8

const (
	opObserve timelineOpKind = iota + 1
	opOpen
	opReset
	opFinish
)

type timelineOp struct {
	kind   timelineOpKind
	screen timelineScreen
	millis int
}

func observeOp(screen timelineScreen, millis int) timelineOp {
	return timelineOp{kind: opObserve, screen: screen, millis: millis}
}

func openOp(screen timelineScreen) timelineOp {
	return timelineOp{kind: opOpen, screen: screen}
}

type timelineCheckKind uint8

const (
	checkTransitions timelineCheckKind = iota + 1
	checkHold
	checkComposerEmpty
	checkNextDeadline
	checkOpened
	checkClosed
	checkRefused
)

type timelineCheck struct {
	kind   timelineCheckKind
	count  int
	reason gateReason
	value  int64
	close  interactionCloseReason
}

func transitionsCheck(count int) timelineCheck {
	return timelineCheck{kind: checkTransitions, count: count}
}
func holdCheck(reason gateReason) timelineCheck {
	return timelineCheck{kind: checkHold, reason: reason}
}
func deadlineCheck(millis int64) timelineCheck {
	return timelineCheck{kind: checkNextDeadline, value: millis}
}
func openedCheck(episode int64) timelineCheck {
	return timelineCheck{kind: checkOpened, value: episode}
}
func closedCheck(reason interactionCloseReason) timelineCheck {
	return timelineCheck{kind: checkClosed, close: reason}
}

var (
	composerEmptyCheck = timelineCheck{kind: checkComposerEmpty}
	refusedCheck       = timelineCheck{kind: checkRefused}
)

type timelineStep struct {
	op     timelineOp
	checks []timelineCheck
}

func TestRuntimeObservation_RuntimeEpisodeTimelinesOpenHoldAndCloseOnTheirBoundaries(t *testing.T) {
	t.Parallel()
	if _, err := newEpisodeTracker(0); err == nil {
		t.Fatal("a tracker numbering episodes from zero is refused")
	}
	const (
		disposable = "write disposable.txt"
		first      = "write first.txt"
		other      = "write other.txt"
		third      = "write third.txt"
	)
	at := func(sequence uint64) opt[uint64] { return some(sequence) }
	fixed := none[uint64]()
	timelines := []struct {
		name  string
		steps []timelineStep
	}{
		{"the first candidate gates before the 100 ms opening boundary", []timelineStep{
			{observeOp(permissionScreen(disposable, fixed), 0), []timelineCheck{transitionsCheck(0), holdCheck(gateReasonCandidate), deadlineCheck(100)}},
			{observeOp(permissionScreen(disposable, fixed), 99), []timelineCheck{transitionsCheck(0)}},
			{observeOp(permissionScreen(disposable, fixed), 100), []timelineCheck{transitionsCheck(1), openedCheck(1), holdCheck(gateReasonInteraction)}},
		}},
		{"an incompatible initial candidate restarts stability", []timelineStep{
			{observeOp(permissionScreen(first, at(1)), 0), nil},
			{observeOp(permissionScreen(other, at(2)), 99), []timelineCheck{transitionsCheck(0)}},
			{observeOp(permissionScreen(other, at(2)), 100), []timelineCheck{transitionsCheck(0)}},
			{observeOp(permissionScreen(other, at(2)), 199), []timelineCheck{transitionsCheck(1), openedCheck(1)}},
		}},
		{"a return to the original cancels the replacement and a third form restarts it", []timelineStep{
			{openOp(permissionScreen(first, at(1))), nil},
			{observeOp(permissionScreen(other, at(2)), 200), nil},
			{observeOp(permissionScreen(first, at(3)), 250), []timelineCheck{transitionsCheck(0)}},
			{observeOp(permissionScreen(other, at(4)), 300), []timelineCheck{transitionsCheck(0)}},
			{observeOp(permissionScreen(third, at(5)), 350), []timelineCheck{transitionsCheck(0)}},
			{observeOp(permissionScreen(third, at(5)), 400), []timelineCheck{transitionsCheck(0)}},
			{observeOp(permissionScreen(third, at(5)), 450), []timelineCheck{transitionsCheck(2)}},
		}},
		{"a verified composer stays held until the 250 ms episode closure", []timelineStep{
			{openOp(permissionScreen(disposable, at(1))), nil},
			{observeOp(composerScreen(at(2)), 200), []timelineCheck{transitionsCheck(0), holdCheck(gateReasonInteraction), deadlineCheck(450)}},
			{observeOp(composerScreen(at(2)), 449), []timelineCheck{transitionsCheck(0), holdCheck(gateReasonInteraction)}},
			{observeOp(composerScreen(at(2)), 450), []timelineCheck{transitionsCheck(1), closedCheck(interactionCloseReasonNotVisible), composerEmptyCheck}},
		}},
		{"uncertainty interrupts the verified boundary", []timelineStep{
			{openOp(permissionScreen(disposable, at(1))), nil},
			{observeOp(composerScreen(at(2)), 200), nil},
			{observeOp(unknownScreen(3), 449), nil},
			{observeOp(composerScreen(at(4)), 450), []timelineCheck{transitionsCheck(0)}},
			{observeOp(composerScreen(at(4)), 699), []timelineCheck{transitionsCheck(0)}},
			{observeOp(composerScreen(at(4)), 700), []timelineCheck{transitionsCheck(1)}},
		}},
		{"identical requests are distinct only after a verified boundary", []timelineStep{
			{openOp(permissionScreen(disposable, at(1))), nil},
			{observeOp(permissionScreen(disposable, at(2)), 200), []timelineCheck{transitionsCheck(0)}},
			{observeOp(composerScreen(at(3)), 300), nil},
			{observeOp(composerScreen(at(3)), 550), nil},
			{observeOp(permissionScreen(disposable, at(4)), 600), []timelineCheck{transitionsCheck(0)}},
			{observeOp(permissionScreen(disposable, at(4)), 700), []timelineCheck{transitionsCheck(1), openedCheck(2)}},
		}},
		{"a gap reset restarts an uncommitted candidate", []timelineStep{
			{observeOp(permissionScreen(disposable, fixed), 0), nil},
			{timelineOp{kind: opReset}, nil},
			{observeOp(permissionScreen(disposable, fixed), 5000), []timelineCheck{transitionsCheck(0)}},
			{observeOp(permissionScreen(disposable, fixed), 5100), []timelineCheck{transitionsCheck(1)}},
		}},
		{"a gap reset restarts closure debounce and keeps the active episode held", []timelineStep{
			{openOp(permissionScreen(disposable, at(1))), nil},
			{observeOp(composerScreen(at(2)), 200), nil},
			{timelineOp{kind: opReset}, nil},
			{observeOp(composerScreen(at(2)), 5000), []timelineCheck{transitionsCheck(0), holdCheck(gateReasonInteraction)}},
			{observeOp(composerScreen(at(2)), 5249), []timelineCheck{transitionsCheck(0), holdCheck(gateReasonInteraction)}},
			{observeOp(composerScreen(at(2)), 5250), []timelineCheck{transitionsCheck(1), composerEmptyCheck}},
		}},
		{"finish closes the active episode once with process exit", []timelineStep{
			{openOp(permissionScreen(disposable, fixed)), nil},
			{timelineOp{kind: opFinish}, []timelineCheck{transitionsCheck(1), closedCheck(interactionCloseReasonProcessExited)}},
			{timelineOp{kind: opFinish}, []timelineCheck{transitionsCheck(0)}},
		}},
		{"an empty composer gates input without allocating rows", []timelineStep{
			{observeOp(composerScreen(fixed), 0), []timelineCheck{transitionsCheck(0), composerEmptyCheck}},
		}},
		{"an unknown screen holds input without allocating rows", []timelineStep{
			{observeOp(unknownScreen(1), 0), []timelineCheck{holdCheck(gateReasonUnknown)}},
		}},
		{"a regressing output sequence is refused", []timelineStep{
			{observeOp(permissionScreen(disposable, at(2)), 0), nil},
			{observeOp(permissionScreen(disposable, at(1)), 100), []timelineCheck{refusedCheck}},
		}},
	}
	for _, timeline := range timelines {
		origin := time.Now()
		tracker := newTestTracker(t)
		for index, step := range timeline.steps {
			context := fmt.Sprintf("%s: step %d", timeline.name, index)
			var update episodeUpdate
			var err error
			switch step.op.kind {
			case opObserve:
				value := step.op.screen.observation(t)
				update, err = tracker.observe(&value, origin.Add(time.Duration(step.op.millis)*time.Millisecond), observationTrackerNow)
			case opOpen:
				update = openEpisode(t, tracker, step.op.screen.observation(t), origin)
			case opReset:
				if err := tracker.resetObservation(); err != nil || len(step.checks) != 0 {
					t.Fatalf("%s: a reset yields no update: %v", context, err)
				}
				continue
			case opFinish:
				update, err = tracker.finish(observationTrackerNow)
			}
			if slices.ContainsFunc(step.checks, func(c timelineCheck) bool { return c.kind == checkRefused }) {
				if err == nil {
					t.Fatalf("%s is refused", context)
				}
				continue
			}
			if err != nil {
				t.Fatalf("%s: %v", context, err)
			}
			for _, check := range step.checks {
				switch check.kind {
				case checkTransitions:
					if len(update.transitions) != check.count {
						t.Fatalf("%s: %d transitions, want %d", context, len(update.transitions), check.count)
					}
				case checkHold:
					if update.eligibility != (inputEligibilityHold{reason: check.reason}) {
						t.Fatalf("%s: eligibility %v", context, update.eligibility)
					}
				case checkComposerEmpty:
					if update.eligibility != (inputEligibilityComposerEmpty{}) {
						t.Fatalf("%s: eligibility %v", context, update.eligibility)
					}
				case checkNextDeadline:
					if update.nextDeadline != some(origin.Add(time.Duration(check.value)*time.Millisecond)) {
						t.Fatalf("%s: deadline %v", context, update.nextDeadline)
					}
				case checkOpened:
					if update.transitions[0].kind != runtimeEventKindInteractionOpened || update.transitions[0].episodeSeq != some(check.value) {
						t.Fatalf("%s: %+v", context, update.transitions[0])
					}
				case checkClosed:
					if update.transitions[0].kind != runtimeEventKindInteractionClosed || update.transitions[0].closeReason != some(check.close) {
						t.Fatalf("%s: %+v", context, update.transitions[0])
					}
				case checkRefused:
					panic("handled before the update")
				}
			}
		}
	}
}

func TestRuntimeObservation_RuntimeEpisodeClassAndStableChoiceChangesSupersedeAfterStability(t *testing.T) {
	t.Parallel()
	for _, change := range []string{"operation", "class", "choice"} {
		origin := time.Now()
		tracker := newTestTracker(t)
		openEpisode(t, tracker, sequenced(permissionObservation(t, "write disposable.txt"), 1), origin)
		definition := syntheticDefinitions()
		rows := permissionLines("write disposable.txt")
		var next screenObservation
		switch change {
		case "operation":
			next = sequenced(permissionObservation(t, "write other.txt"), 2)
		case "class":
			jset(definition, "question", "codex", "profile", "rules", 0, "observation")
			snapshot := syntheticTerminal(t, rows, 80, false).snapshot()
			next = sequenced(syntheticObservation(t, definition, &snapshot), 2)
		case "choice":
			jset(definition, "^> (?P<label>Allow this request)$", "codex", "profile", "rules", 0, "matchers", 1, "pattern")
			rows[2] = "> Allow this request"
			snapshot := syntheticTerminal(t, rows, 80, false).snapshot()
			next = sequenced(syntheticObservation(t, definition, &snapshot), 2)
		}
		pending := mustObserve(t, tracker, next, origin.Add(200*time.Millisecond))
		if len(pending.transitions) != 0 {
			t.Fatalf("%s: pending %+v", change, pending)
		}
		if change == "operation" {
			if _, held := pending.eligibility.(inputEligibilityHold); !held {
				t.Fatalf("eligibility %v", pending.eligibility)
			}
			if update := mustObserve(t, tracker, next, origin.Add(299*time.Millisecond)); len(update.transitions) != 0 {
				t.Fatalf("before stability: %+v", update)
			}
		}
		stable := mustObserve(t, tracker, next, origin.Add(300*time.Millisecond))
		class := interactionClassPermissionRequest
		if change == "class" {
			class = interactionClassQuestion
		}
		if len(stable.transitions) != 2 {
			t.Fatalf("%s: %d transitions", change, len(stable.transitions))
		}
		closed, opened := stable.transitions[0], stable.transitions[1]
		if closed.kind != runtimeEventKindInteractionClosed || closed.closeReason != some(interactionCloseReasonSuperseded) || closed.episodeSeq != some(int64(1)) ||
			opened.kind != runtimeEventKindInteractionOpened || opened.episodeSeq != some(int64(2)) || opened.classification != some(class) {
			t.Fatalf("%s: %+v", change, stable.transitions)
		}
	}
}

func TestRuntimeObservation_RuntimeEpisodeEquivalentRedrawSchedulesNoRefreshAndNoOpening(t *testing.T) {
	t.Parallel()
	origin := time.Now()
	tracker := newTestTracker(t)
	openEpisode(t, tracker, sequenced(permissionObservation(t, "write disposable.txt"), 1), origin)
	snapshot := syntheticTerminal(t, permissionLines("write disposable.txt"), 100, false).snapshot()
	redraw := syntheticObservation(t, syntheticDefinitions(), &snapshot)
	for _, step := range []struct {
		sequence uint64
		millis   int
	}{{2, 200}, {3, 300}} {
		update := mustObserve(t, tracker, sequenced(redraw, step.sequence), origin.Add(time.Duration(step.millis)*time.Millisecond))
		if len(update.transitions) != 0 || update.visibility != visibilityVisible {
			t.Fatalf("%+v: %+v", step, update)
		}
	}
}

func TestRuntimeObservation_RuntimeEpisodeMetadataRemovesEachScreenFieldAndLeavesNoRedrawRefresh(t *testing.T) {
	t.Parallel()
	const secret = "synthetic-secret-194"
	origin := time.Now()
	tracker := newTestTracker(t)
	opened := openEpisode(t, tracker, secretObservation(t, secret, 80, 1), origin)
	evidence := opened.transitions[0].evidenceJSON
	for _, field := range []string{"operation", "choices", "excerpt"} {
		if string(jsonAt(t, evidence, field)) != "null" {
			t.Fatalf("%s = %s", field, jsonAt(t, evidence, field))
		}
	}
	if strings.Contains(string(evidence), secret) {
		t.Fatal("the opening evidence holds no screen text")
	}
	event, err := materialize(opened.transitions[0], 1)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(mustJSON(t, event)), secret) {
		t.Fatal("the materialized event holds no screen text")
	}
	for _, step := range []struct {
		sequence uint64
		after    time.Duration
		detail   string
	}{{2, 300 * time.Millisecond, "first"}, {3, 400 * time.Millisecond, secret}} {
		redraw := secretObservation(t, secret, 100, step.sequence)
		redraw.rawEvidence.excerpt = some("Detail: " + step.detail)
		if update := mustObserve(t, tracker, redraw, origin.Add(step.after)); len(update.transitions) != 0 {
			t.Fatalf("%+v: %+v", step, update)
		}
	}
}

func TestRuntimeObservation_RuntimeEpisodeMetadataDigestContainsNoHashOfHiddenSemantics(t *testing.T) {
	t.Parallel()
	origin := time.Now()
	first := sequenced(permissionObservation(t, "write first-secret.txt"), 1)
	second := sequenced(permissionObservation(t, "write other-secret.txt"), 1)
	if fingerprintsEqual(first.fingerprint, second.fingerprint) {
		t.Fatal("the hidden semantics differ")
	}
	a := openEpisode(t, newTestTracker(t), first, origin).transitions[0].evidenceJSON
	b := openEpisode(t, newTestTracker(t), second, origin).transitions[0].evidenceJSON
	assertJSONEqual(t, a, b, "the evidence")
	assertJSONEqual(t, jsonAt(t, a, "evidence_digest"), jsonAt(t, b, "evidence_digest"), "the digest")
}

func TestRuntimeObservation_RuntimeEpisodeMaterializeNumbersTheDraftAndRefusesANonPositiveSequence(t *testing.T) {
	t.Parallel()
	update := openEpisode(t, newTestTracker(t), permissionObservation(t, "write disposable.txt"), time.Now())
	draft := update.transitions[0]
	for _, sequence := range []int64{1, 2, 513} {
		event, err := materialize(draft, sequence)
		if err != nil || event.EventSeq != sequence || event.ObservedAt != draft.observedAt || !jsonEqual(event.EvidenceJSON, draft.evidenceJSON) {
			t.Fatalf("%d: %+v %v", sequence, event, err)
		}
	}
	for _, sequence := range []int64{0, -1} {
		if _, err := materialize(draft, sequence); err == nil {
			t.Fatalf("%d is refused", sequence)
		}
	}
}

// trackerRun replays one backend's bundled fixtures through a tracker at 50 ms steps, numbering
// every frame so each observation carries a fresh screen sequence.
type trackerRun struct {
	t        *testing.T
	tracker  *episodeTracker
	origin   time.Time
	sequence uint64
	observe  func(t *testing.T, id string) screenObservation
}

func newTrackerRun(t *testing.T, observe func(t *testing.T, id string) screenObservation) *trackerRun {
	t.Helper()
	return &trackerRun{t: t, tracker: newTestTracker(t), origin: time.Now(), observe: observe}
}

func (r *trackerRun) episode(id string, from, until int) []transitionDraft {
	r.t.Helper()
	var episodes []transitionDraft
	for millis := from; millis <= until; millis += 50 {
		r.sequence++
		value := sequenced(r.observe(r.t, id), r.sequence)
		for _, transition := range mustObserve(r.t, r.tracker, value, r.origin.Add(time.Duration(millis)*time.Millisecond)).transitions {
			if transition.kind == runtimeEventKindInteractionOpened || transition.kind == runtimeEventKindInteractionClosed {
				episodes = append(episodes, transition)
			}
		}
	}
	return episodes
}

func bundledObserver(backend codingAgent, version string) func(t *testing.T, id string) screenObservation {
	return func(t *testing.T, id string) screenObservation {
		t.Helper()
		fixture := bundledFixture(id)
		screen := replayNativeFixture(fixture, func(*[]string) {}, fixtureAlternate(parseTestJSON(fixture)), none[[2]uint16]())
		value, err := newDetector(profileCatalogBundled(), backend, some(version)).observe(&screen)
		if err != nil {
			t.Fatal(err)
		}
		return value
	}
}

func assertDrafts(t *testing.T, drafts []transitionDraft, count int, context string) {
	t.Helper()
	if len(drafts) != count {
		t.Fatalf("%s: %d drafts %+v, want %d", context, len(drafts), drafts, count)
	}
}

func draftRule(t *testing.T, draft transitionDraft) string {
	t.Helper()
	return decodedString(t, jsonAt(t, draft.evidenceJSON, "rule_id"))
}

func TestRuntimeObservation_BundledClaudeAutoTracker_TrackerOpensOnePermissionEpisodeForTheTipFormAndClosesItOnceTheCommandRuns(t *testing.T) {
	t.Parallel()
	run := newTrackerRun(t, bundledObserver(codingAgentClaude, "2.1.302"))
	form := run.episode("claude-2.1.302-access-permission-1", 0, 300)
	assertDrafts(t, form, 1, "form")
	if form[0].kind != runtimeEventKindInteractionOpened || form[0].classification != some(interactionClassPermissionRequest) {
		t.Fatalf("form = %+v", form[0])
	}
	assertDrafts(t, run.episode("claude-2.1.302-access-allow-pre-stage-1", 350, 600), 0, "pre-stage")
	running := run.episode("claude-2.1.302-access-allow-running-1", 650, 1100)
	assertDrafts(t, running, 1, "running")
	if running[0].kind != runtimeEventKindInteractionClosed || running[0].episodeSeq != form[0].episodeSeq || running[0].closeReason != some(interactionCloseReasonNotVisible) {
		t.Fatalf("running = %+v", running[0])
	}
}

func TestRuntimeObservation_BundledClaude21312Tracker_TrackerReplacesThePendingNoticeWithDialogAAfterTheDraftIsCleared(t *testing.T) {
	t.Parallel()
	run := newTrackerRun(t, bundledObserver(codingAgentClaude, "2.1.312"))
	notice := run.episode("claude-2.1.312-question-pending-notice-1", 0, 300)
	assertDrafts(t, notice, 1, "notice")
	if notice[0].kind != runtimeEventKindInteractionOpened || notice[0].classification != some(interactionClassQuestion) || draftRule(t, notice[0]) != "claude-question-pending-notice" {
		t.Fatalf("notice = %+v", notice[0])
	}
	cleared := run.episode("claude-2.1.312-question-pending-after-clear-1", 350, 900)
	assertDrafts(t, cleared, 2, "cleared")
	if cleared[0].kind != runtimeEventKindInteractionClosed || cleared[0].episodeSeq != notice[0].episodeSeq ||
		cleared[1].kind != runtimeEventKindInteractionOpened || cleared[1].classification != some(interactionClassQuestion) ||
		draftRule(t, cleared[1]) != "claude-auto-default-nudge" || cleared[1].episodeSeq == notice[0].episodeSeq {
		t.Fatalf("cleared = %+v", cleared)
	}
}

func TestRuntimeObservation_BundledClaude21312Tracker_TrackerClosesThePendingNoticeWithoutDialogAAfterTheDraftIsSent(t *testing.T) {
	t.Parallel()
	run := newTrackerRun(t, bundledObserver(codingAgentClaude, "2.1.312"))
	notice := run.episode("claude-2.1.312-question-pending-send-notice-1", 0, 300)
	assertDrafts(t, notice, 1, "notice")
	if draftRule(t, notice[0]) != "claude-question-pending-notice" {
		t.Fatalf("notice = %+v", notice[0])
	}
	sent := run.episode("claude-2.1.312-question-pending-after-send-1", 350, 900)
	assertDrafts(t, sent, 1, "sent")
	if sent[0].kind != runtimeEventKindInteractionClosed || sent[0].episodeSeq != notice[0].episodeSeq || sent[0].closeReason != some(interactionCloseReasonNotVisible) {
		t.Fatalf("sent = %+v", sent[0])
	}
	assertDrafts(t, run.episode("claude-2.1.312-question-pending-after-send-idle-1", 950, 1400), 0, "no Dialog A episode opens")
}

func TestRuntimeObservation_BundledClaude21312Tracker_TrackerClosesDialogAOnceItsDeclineReturnsTheComposer(t *testing.T) {
	t.Parallel()
	run := newTrackerRun(t, bundledObserver(codingAgentClaude, "2.1.312"))
	opened := run.episode("claude-2.1.312-auto-default-decline-pre-stage-1", 0, 300)
	assertDrafts(t, opened, 1, "opened")
	assertDrafts(t, run.episode("claude-2.1.312-auto-default-decline-post-stage-1-1", 350, 700), 0, "a focus move keeps the episode")
	closed := run.episode("claude-2.1.312-auto-default-decline-continuation-1", 750, 1300)
	assertDrafts(t, closed, 1, "closed")
	if closed[0].kind != runtimeEventKindInteractionClosed || closed[0].episodeSeq != opened[0].episodeSeq {
		t.Fatalf("closed = %+v", closed[0])
	}
}

func TestRuntimeObservation_BundledClaude21312Tracker_TrackerSupersedesTheFirstBatchedReadWithTheSecondAfterItsAllow(t *testing.T) {
	t.Parallel()
	run := newTrackerRun(t, bundledObserver(codingAgentClaude, "2.1.312"))
	first := run.episode("claude-2.1.312-read-file-batched-permission-1", 0, 300)
	assertDrafts(t, first, 1, "first")
	if first[0].kind != runtimeEventKindInteractionOpened || draftRule(t, first[0]) != "claude-read-file-permission" {
		t.Fatalf("first = %+v", first[0])
	}
	second := run.episode("claude-2.1.312-read-file-batched-allow-post-stage-1", 350, 900)
	assertDrafts(t, second, 2, "second")
	if second[0].kind != runtimeEventKindInteractionClosed || second[0].closeReason != some(interactionCloseReasonSuperseded) || second[0].episodeSeq != first[0].episodeSeq ||
		second[1].kind != runtimeEventKindInteractionOpened || draftRule(t, second[1]) != "claude-read-file-permission" || second[1].episodeSeq == first[0].episodeSeq {
		t.Fatalf("second = %+v", second)
	}
}

func TestRuntimeObservation_BundledClaudeAskUserQuestionTracker_TrackerKeepsOneQuestionEpisodeAcrossTheFormAndClosesItOnTheIdleComposer(t *testing.T) {
	t.Parallel()
	run := newTrackerRun(t, bundledObserver(codingAgentClaude, "2.1.322"))
	opened := run.episode("claude-2.1.322-ask-tabs-tmux-120-1", 0, 300)
	assertDrafts(t, opened, 1, "opened")
	if opened[0].kind != runtimeEventKindInteractionOpened || opened[0].classification != some(interactionClassQuestion) || draftRule(t, opened[0]) != "claude-ask-user-question" {
		t.Fatalf("opened = %+v", opened[0])
	}
	at := 350
	for _, id := range []string{"claude-2.1.322-ask-tabs-tmux-120-2", "claude-2.1.322-ask-review-tmux-120-1", "claude-2.1.322-ask-review-tmux-120-2"} {
		assertDrafts(t, run.episode(id, at, at+300), 0, id+" keeps the episode")
		at += 350
	}
	closed := run.episode("claude-2.1.322-ask-single-closed-tmux-120-1", at, at+550)
	assertDrafts(t, closed, 1, "closed")
	if closed[0].kind != runtimeEventKindInteractionClosed || closed[0].episodeSeq != opened[0].episodeSeq || closed[0].closeReason != some(interactionCloseReasonNotVisible) {
		t.Fatalf("closed = %+v", closed[0])
	}
}

func TestRuntimeObservation_BundledClaudeAskUserQuestionTracker_TrackerOpensTheReviewScreenAsTheSameQuestionClass(t *testing.T) {
	t.Parallel()
	run := newTrackerRun(t, bundledObserver(codingAgentClaude, "2.1.322"))
	opened := run.episode("claude-2.1.322-ask-review-tmux-120-2", 0, 300)
	assertDrafts(t, opened, 1, "opened")
	if opened[0].classification != some(interactionClassQuestion) || draftRule(t, opened[0]) != "claude-ask-user-question-review" {
		t.Fatalf("opened = %+v", opened[0])
	}
}

func TestRuntimeObservation_BundledClaudeAskUserQuestionTracker_TrackerContinuesThePendingNoticeIntoTheFormInOneEpisode(t *testing.T) {
	t.Parallel()
	run := newTrackerRun(t, bundledObserver(codingAgentClaude, "2.1.322"))
	notice := run.episode("claude-2.1.312-question-pending-notice-1", 0, 300)
	assertDrafts(t, notice, 1, "notice")
	if notice[0].classification != some(interactionClassQuestion) || draftRule(t, notice[0]) != "claude-question-pending-notice" {
		t.Fatalf("notice = %+v", notice[0])
	}
	assertDrafts(t, run.episode("claude-2.1.322-ask-pending-tmux-120-1", 350, 900), 0, "the form continues the notice's episode")
}

const controllerFixtureVersion = "0.0.1-synthetic"

func refreshWriterRun(t *testing.T, f *writerFixture) {
	t.Helper()
	f.run = mustRun(t, f.conn, f.run.runID)
}

// configureController records the synthetic native version, tested coverage and this process as
// the runner.
func configureController(t *testing.T, f *writerFixture) {
	t.Helper()
	if _, err := f.conn.execute("UPDATE member_runs SET native_version=?1,coverage='tested',runner_pid=owner_pid,runner_start_identity=owner_start_identity WHERE run_id=?2", controllerFixtureVersion, f.run.runID); err != nil {
		t.Fatal(err)
	}
	refreshWriterRun(t, f)
}

func markRunning(t *testing.T, f *writerFixture, child processIdentity) {
	t.Helper()
	if _, err := f.conn.execute("UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at=?3 WHERE run_id=?4", child.pid, child.startIdentity, observationFixtureNow, f.run.runID); err != nil {
		t.Fatal(err)
	}
	refreshWriterRun(t, f)
}

func controllerForm(operation string) []byte {
	return []byte("\x1b[2J\x1b[19;1HSynthetic permission request\r\nOperation: " + operation + "\r\n> Allow once\r\n  Deny once\r\nRequest form: synthetic-only\r\nSelection: ")
}

// testChild is a /bin/cat native child this test owns, reaped by a waiter goroutine.
type testChild struct {
	cmd      *exec.Cmd
	identity processIdentity
	exited   chan *os.ProcessState
}

func spawnCat(t *testing.T) *testChild {
	t.Helper()
	cmd := exec.Command("/bin/cat")
	if _, err := cmd.StdinPipe(); err != nil {
		t.Fatal(err)
	}
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	child := &testChild{cmd: cmd, exited: make(chan *os.ProcessState, 1)}
	go func() {
		state, _ := cmd.Process.Wait()
		child.exited <- state
	}()
	t.Cleanup(func() {
		_ = cmd.Process.Kill()
		<-child.exited
	})
	child.identity = requireIdentity(t, int64(cmd.Process.Pid))
	return child
}

func (c *testChild) alive(t *testing.T) bool {
	t.Helper()
	alive, err := c.identity.matches()
	if err != nil {
		t.Fatal(err)
	}
	return alive
}

// terminate sends SIGTERM and waits for the signal exit, keeping the state for the cleanup.
func (c *testChild) terminate(t *testing.T) {
	t.Helper()
	if err := c.cmd.Process.Signal(syscall.SIGTERM); err != nil {
		t.Fatal(err)
	}
	select {
	case state := <-c.exited:
		c.exited <- state
		if status, ok := state.Sys().(syscall.WaitStatus); !ok || !status.Signaled() || status.Signal() != syscall.SIGTERM {
			t.Fatalf("exit = %v", state)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("owned child signal exit")
	}
}

func newController(t *testing.T, run runRecord, catalog *profileCatalog) *observationController {
	t.Helper()
	controller, err := newObservationController(run, catalog)
	if err != nil {
		t.Fatal(err)
	}
	return controller
}

// startedController is a controller started for a running /bin/cat native child on a 24x80
// terminal.
func startedController(t *testing.T, f *writerFixture, writer *eventWriter, c clock, catalog *profileCatalog) (*observationController, *testChild, *terminalState) {
	t.Helper()
	controller := newController(t, f.run, catalog)
	child := spawnCat(t)
	markRunning(t, f, child.identity)
	terminal := mustTerminalState(t, terminalSize{rows: 24, cols: 80})
	snapshot := terminal.snapshot()
	if err := controller.start(f.run, &snapshot, writer, c); err != nil {
		t.Fatal(err)
	}
	return controller, child, terminal
}

func observeSnapshot(t *testing.T, controller *observationController, writer *eventWriter, c clock, terminal *terminalState) {
	t.Helper()
	snapshot := terminal.snapshot()
	if err := controller.observer(writer, c).observe(&snapshot); err != nil {
		t.Fatal(err)
	}
}

func pollStatus(t *testing.T, controller *observationController, writer *eventWriter, c clock, terminal *terminalState) observationStatus {
	t.Helper()
	snapshot := terminal.snapshot()
	status, err := controller.poll(&snapshot, writer, c)
	if err != nil {
		t.Fatal(err)
	}
	return status
}

func feedObservedBy(t *testing.T, terminal *terminalState, data []byte, controller *observationController, writer *eventWriter, c clock) {
	t.Helper()
	if err := terminal.feedObserved(data, controller.observer(writer, c)); err != nil {
		t.Fatal(err)
	}
}

func runEvents(t *testing.T, f *writerFixture) []runtimeEventView {
	t.Helper()
	var views []runtimeEventView
	for _, row := range tableRows(t, f.conn, "SELECT event_id FROM member_runtime_events WHERE run_id=?1 ORDER BY event_seq", f.run.runID) {
		views = append(views, readEvent(t, f.conn, row[0].(int64)))
	}
	return views
}

func eventKinds(t *testing.T, f *writerFixture) []string {
	t.Helper()
	var kinds []string
	for _, row := range tableRows(t, f.conn, "SELECT kind FROM member_runtime_events WHERE run_id=?1 ORDER BY event_seq", f.run.runID) {
		kinds = append(kinds, row[0].(string))
	}
	return kinds
}

func openedCount(t *testing.T, f *writerFixture) int64 {
	t.Helper()
	return queryInt64(t, f.conn, "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='interaction_opened'", f.run.runID)
}

func eventViewAt(t *testing.T, f *writerFixture, sequence int64) runtimeEventView {
	t.Helper()
	return readEvent(t, f.conn, queryInt64(t, f.conn, "SELECT event_id FROM member_runtime_events WHERE run_id=?1 AND event_seq=?2", f.run.runID, sequence))
}

func observerInitialized(t *testing.T, backend string, nativeVersion opt[string], cov coverage) error {
	t.Helper()
	f := newWriterFixture(t)
	if _, err := f.conn.execute("UPDATE member_runs SET backend=?1,native_version=?2,coverage=?3 WHERE run_id=?4", backend, nativeVersion, cov.asStr(), f.run.runID); err != nil {
		t.Fatal(err)
	}
	refreshWriterRun(t, f)
	_, err := newObservationController(f.run, mustLoadCatalog(t, inspectionFixtureDefinition()))
	return err
}

func TestRuntimeObservation_ObserverInitialization_ObserverInitializationRequiresTheBackendsProfileDetectionCoverage(t *testing.T) {
	t.Parallel()
	synthetic := some(controllerFixtureVersion)
	cases := []struct {
		backend  string
		version  opt[string]
		coverage coverage
		accepted bool
	}{
		{"codex", synthetic, coverageTested, true},
		{"codex", some("0.0.2-synthetic"), coverageTested, true},
		{"codex", some("0.0.0-below"), coverageTested, true},
		{"codex", none[string](), coverageTested, true},
		{"codex", synthetic, coverageUnknown, false},
		{"codex", none[string](), coverageUnknown, false},
		{"claude", synthetic, coverageUnknown, true},
		{"claude", none[string](), coverageUnknown, true},
		{"claude", synthetic, coverageTested, false},
		{"claude", none[string](), coverageTested, false},
	}
	for _, tc := range cases {
		if err := observerInitialized(t, tc.backend, tc.version, tc.coverage); (err == nil) != tc.accepted {
			t.Fatalf("%s %v %s: %v", tc.backend, tc.version, tc.coverage.asStr(), err)
		}
	}
}

func assertAuthorityFence(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		message := cafleetErrorOf(err).message()
		if !strings.Contains(message, "generation") && !strings.Contains(message, "exhaust") {
			t.Fatalf("explicit authority fence: %v", err)
		}
	}
}

func TestRuntimeObservation_InspectionTests_PermissionInspectionRequestGenerationExhaustionInvalidatesWithoutWrap(t *testing.T) {
	t.Parallel()
	until := time.Now().Add(5 * time.Second)
	c := observationFixtureClock(t)
	nowMs := 0
	f := newWriterFixture(t)
	if _, err := f.conn.execute("UPDATE member_runs SET native_version='0.0.1-synthetic',coverage='tested' WHERE run_id=?1", f.run.runID); err != nil {
		t.Fatal(err)
	}
	refreshWriterRun(t, f)
	catalog := mustLoadCatalog(t, inspectionFixtureDefinition())
	controller := newController(t, f.run, catalog)
	writer, err := f.start()
	if err != nil {
		t.Fatal(err)
	}
	writer.setDeadline(until)
	child := spawnCat(t)
	markRunning(t, f, child.identity)
	terminal := mustTerminalState(t, terminalSize{rows: 24, cols: 80})
	initial := terminal.snapshot()
	if err := controller.start(f.run, &initial, writer, c); err != nil {
		t.Fatal(err)
	}
	mustFeed(t, terminal, inspectionFixtureForm("overflow current request", "synthetic-only"))
	observeSnapshot(t, controller, writer, c, terminal)
	for {
		pollStatus(t, controller, writer, c, terminal)
		if controller.currentRequest != nil {
			break
		}
		if !time.Now().Before(until) {
			t.Fatal("actual current authority before the original fixture deadline")
		}
		nowMs += 2
		c.setElapsed(time.Duration(nowMs) * time.Millisecond)
	}
	if controller.tracker.activeEpisode() == none[episodeRef]() {
		t.Fatal("an episode is active")
	}
	controller.requestGeneration = math.MaxUint64
	if controller.observerFailed(observationFailureParser, c) == nil {
		t.Fatal("an exhausted generation refuses the failure")
	}
	if controller.requestGeneration != math.MaxUint64 || controller.currentRequest != nil {
		t.Fatal("the exhausted generation clears the authority without wrapping")
	}
	mustFeed(t, terminal, inspectionFixtureForm("overflow current request", "synthetic-only"))
	snapshot := terminal.snapshot()
	observed, err := newDetector(catalog, codingAgentCodex, some(controllerFixtureVersion)).observe(&snapshot)
	if err != nil || observed.matchEvidence != matchEvidenceComplete || observed.kind != observationKindPermissionRequest {
		t.Fatalf("observed %+v %v", observed, err)
	}
	assertAuthorityFence(t, controller.observer(writer, c).observe(&snapshot))
	if controller.requestGeneration != math.MaxUint64 || controller.currentRequest != nil {
		t.Fatal("a later genuine complete screen cannot restore exhausted authority")
	}
	nowMs += 110
	c.setElapsed(time.Duration(nowMs) * time.Millisecond)
	_, err = controller.poll(&snapshot, writer, c)
	assertAuthorityFence(t, err)
	if controller.requestGeneration != math.MaxUint64 || controller.currentRequest != nil {
		t.Fatal("stable later complete evidence stays fenced")
	}
	if controller.observerFailed(observationFailureDetector, c) == nil || controller.requestGeneration != math.MaxUint64 || controller.currentRequest != nil {
		t.Fatal("a later detector failure stays fenced")
	}
	pending, unsaved := writer.finish(until)
	if status := writer.status(); pending != 0 || unsaved != 0 || status.diagnostic != none[string]() {
		t.Fatalf("writer cleanup (%d, %d) %+v", pending, unsaved, status)
	}
}

// opencodeForm is the bundled OpenCode fixture id with its command rows replaced by command, each
// padded to the command box with the scrollbar column trailerCol cells in.
func resizedOpencodeForm(t *testing.T, id string, firstRow int, command [2]string, trailerCol int) []byte {
	t.Helper()
	screen := parseTestJSON(bundledScreen(id))
	number, ok := jget(screen, "cols").(jsonNumber)
	if !ok {
		t.Fatal("the fixture names its columns")
	}
	cols, err := strconv.Atoi(string(number))
	if err != nil {
		t.Fatal(err)
	}
	for offset, text := range command {
		row := text + strings.Repeat(" ", trailerCol-utf8.RuneCountInString(text)) + "█" + strings.Repeat(" ", cols-trailerCol-1)
		jset(screen, row, "lines", firstRow+offset)
	}
	return nativeFixtureBytes(testJSONText(screen))
}

func TestRuntimeObservation_InspectionTests_OpencodeResizeAcrossWSplitStartsANewGenerationAndARewrapANewEpisode(t *testing.T) {
	t.Parallel()
	until := time.Now().Add(5 * time.Second)
	c := observationFixtureClock(t)
	nowMs := 0
	f := newWriterFixture(t)
	if _, err := f.conn.execute("UPDATE member_runs SET backend='opencode',native_version='2.1.4',coverage='tested' WHERE run_id=?1", f.run.runID); err != nil {
		t.Fatal(err)
	}
	refreshWriterRun(t, f)
	controller := newController(t, f.run, profileCatalogBundled())
	writer, err := f.start()
	if err != nil {
		t.Fatal(err)
	}
	writer.setDeadline(until)
	child := spawnCat(t)
	markRunning(t, f, child.identity)
	command := [2]string{"  $ /tmp/x/permission-probe allow", ""}
	wide := resizedOpencodeForm(t, "opencode-2.1.4-ask-permission-1", 14, command, 116)
	split := resizedOpencodeForm(t, "opencode-2.1.4-narrow-permission-057-1", 27, command, 53)
	rewrapped := resizedOpencodeForm(t, "opencode-2.1.4-narrow-permission-057-1", 27, [2]string{"  $ /tmp/x/", "  permission-probe allow"}, 53)
	terminal := mustTerminalState(t, terminalSize{rows: 36, cols: 120})
	initial := terminal.snapshot()
	if err := controller.start(f.run, &initial, writer, c); err != nil {
		t.Fatal(err)
	}
	activeSequence := func() opt[int64] {
		if episode, ok := controller.tracker.activeEpisode().get(); ok {
			return some(episode.sequence)
		}
		return none[int64]()
	}
	show := func(data []byte, settled func() bool) (uint64, opt[int64]) {
		mustFeed(t, terminal, data)
		observeSnapshot(t, controller, writer, c, terminal)
		for {
			pollStatus(t, controller, writer, c, terminal)
			if settled() {
				break
			}
			if !time.Now().Before(until) {
				t.Fatal("the observed state settles in time")
			}
			nowMs += 2
			c.setElapsed(time.Duration(nowMs) * time.Millisecond)
		}
		return controller.requestGeneration, activeSequence()
	}
	wideGeneration, wideEpisode := show(wide, func() bool { return controller.currentRequest != nil })
	wideSequence, ok := wideEpisode.get()
	if !ok {
		t.Fatal("the wide form opens an episode")
	}
	mustResize(t, terminal, terminalSize{rows: 36, cols: 57})
	splitGeneration, splitEpisode := show(split, func() bool {
		return controller.currentRequest != nil && controller.requestGeneration > wideGeneration
	})
	if splitGeneration <= wideGeneration || splitEpisode != wideEpisode {
		t.Fatalf("the resize starts a new request generation and keeps the episode: %d/%d %v/%v", splitGeneration, wideGeneration, splitEpisode, wideEpisode)
	}
	rewrapGeneration, rewrapEpisode := show(rewrapped, func() bool {
		sequence, active := activeSequence().get()
		return controller.currentRequest != nil && active && sequence > wideSequence
	})
	rewrapSequence, _ := rewrapEpisode.get()
	if rewrapGeneration <= splitGeneration || rewrapSequence <= wideSequence {
		t.Fatalf("a moved row break opens a new episode: %d %v", rewrapGeneration, rewrapEpisode)
	}
	if pending, unsaved := writer.finish(until); pending != 0 || unsaved != 0 {
		t.Fatalf("writer cleanup (%d, %d)", pending, unsaved)
	}
}

// detectorFault fails the next detector call at site once armed.
type detectorFault struct {
	site  observationPhase
	armed atomic.Bool
	calls atomic.Uint64
}

func (f *detectorFault) before(site observationPhase, operation observationOperation) error {
	if site == f.site && operation == observationOperationDetector {
		f.calls.Add(1)
		if f.armed.Swap(false) {
			return runtimeError("private detector failure canary")
		}
	}
	return nil
}

func (f *detectorFault) orderShutdownParsingAfterNativeExit(time.Time) bool {
	return false
}

type liveCase struct {
	t          *testing.T
	writer     *eventWriter
	controller *observationController
	hooks      *writerTestHooks
	child      *testChild
	terminal   *terminalState
	clock      *testClock
	fixture    *writerFixture
}

func newLiveCase(t *testing.T, hooks observationHooks) *liveCase {
	t.Helper()
	f := newWriterFixture(t)
	configureController(t, f)
	writerHooks := &writerTestHooks{}
	writer := startReal(t, f, writerHooks)
	controller := newController(t, f.run, syntheticCatalog(t))
	if hooks != nil {
		controller.observationHooks = hooks
	}
	child := spawnCat(t)
	markRunning(t, f, child.identity)
	terminal := mustTerminalState(t, terminalSize{rows: 24, cols: 80})
	c := observationFixtureClock(t)
	initial := terminal.snapshot()
	if err := controller.start(f.run, &initial, writer, c); err != nil {
		t.Fatal(err)
	}
	feedObservedBy(t, terminal, controllerForm("write terminal-order.txt"), controller, writer, c)
	c.setElapsed(100 * time.Millisecond)
	pollStatus(t, controller, writer, c, terminal)
	yieldUntil(t, func() bool { return committed(t, f) == 1 && writer.status().pendingCount == 0 })
	live := &liveCase{t: t, writer: writer, controller: controller, hooks: writerHooks, child: child, terminal: terminal, clock: c, fixture: f}
	t.Cleanup(func() {
		writerHooks.releaseForCleanup()
		writer.finish(time.Now().Add(250 * time.Millisecond))
	})
	return live
}

func (l *liveCase) exit(millis int) uint64 {
	l.t.Helper()
	if !l.child.alive(l.t) {
		l.t.Fatal("the child runs")
	}
	l.child.terminate(l.t)
	l.clock.setElapsed(time.Duration(millis) * time.Millisecond)
	snapshot := l.terminal.snapshot()
	if err := l.controller.childExited(childExitSignal{signal: 15}, &snapshot, l.writer, l.clock); err != nil {
		l.t.Fatal(err)
	}
	return snapshot.screenSeq
}

func (l *liveCase) drain(until time.Time) (uint64, uint64) {
	l.t.Helper()
	l.writer.setDeadline(until)
	for time.Now().Before(until) {
		pollStatus(l.t, l.controller, l.writer, l.clock, l.terminal)
		if !l.controller.hasPending() && l.writer.status().pendingCount == 0 {
			break
		}
		time.Sleep(2 * time.Millisecond)
	}
	return l.writer.finish(until)
}

func assertDetectorGap(t *testing.T, status observationStatus) {
	t.Helper()
	_, held := status.eligibility.(inputEligibilityHold)
	if status.diagnostic != some("detector_unavailable") || !held || status.gapCount == 0 {
		t.Fatalf("status = %+v", status)
	}
}

func assertOpenEpisode(t *testing.T, l *liveCase) {
	t.Helper()
	interaction, ok := runEvents(t, l.fixture)[0].interaction.get()
	if !ok || interaction.episodeSeq != 1 || interaction.closedAt != none[string]() {
		t.Fatalf("interaction = %+v", interaction)
	}
}

func TestRuntimeObservation_ControllerTests_FailureHandling_ObservationFailures_RunnerQuietDetectorFailureReturnsTypedHeldStatusAndPreservesTheEpisode(t *testing.T) {
	t.Parallel()
	fault := &detectorFault{site: observationPhaseQuietTimer}
	l := newLiveCase(t, fault)
	l.clock.setElapsed(200 * time.Millisecond)
	mustFeed(t, l.terminal, controllerForm("write candidate-after-open.txt"))
	observeSnapshot(t, l.controller, l.writer, l.clock, l.terminal)
	sequence := l.terminal.snapshot().screenSeq
	calls := fault.calls.Load()
	fault.armed.Store(true)
	l.clock.setElapsed(300 * time.Millisecond)
	status := pollStatus(t, l.controller, l.writer, l.clock, l.terminal)
	if fault.calls.Load() != calls+1 || fault.armed.Load() {
		t.Fatal("the armed fault fired once")
	}
	assertDetectorGap(t, status)
	if l.terminal.snapshot().screenSeq != sequence || openedCount(t, l.fixture) != 1 || !l.child.alive(t) {
		t.Fatal("the episode and the child are preserved")
	}
	assertOpenEpisode(t, l)
}

func TestRuntimeObservation_ControllerTests_FailureHandling_ObservationFailures_RunnerOrderedAdapterPreservesDetectorAttributionAndBoundedMetadata(t *testing.T) {
	t.Parallel()
	fault := &detectorFault{site: observationPhaseChildOutput}
	l := newLiveCase(t, fault)
	fault.armed.Store(true)
	l.clock.setElapsed(200 * time.Millisecond)
	calls := fault.calls.Load()
	snapshot := l.terminal.snapshot()
	_ = l.controller.observer(l.writer, l.clock).observe(&snapshot) //nolint:errcheck // the attempt's outcome is the status read next
	if fault.calls.Load() != calls+1 || fault.armed.Load() {
		t.Fatal("the armed fault fired once")
	}
	assertDetectorGap(t, l.controller.status())
	if openedCount(t, l.fixture) != 1 || !l.child.alive(t) {
		t.Fatal("the episode and the child are preserved")
	}
	assertOpenEpisode(t, l)
}

func TestRuntimeObservation_ControllerTests_FailureHandling_ObservationFailures_RunnerClockAndOutputSequenceInvariantsPropagateOutsideRecoverableFailures(t *testing.T) {
	t.Parallel()
	for _, invariant := range []string{"clock", "sequence"} {
		l := newLiveCase(t, &detectorFault{site: observationPhaseChildOutput})
		before := l.controller.status().gapCount
		var err error
		if invariant == "clock" {
			l.clock.setElapsed(99 * time.Millisecond)
			snapshot := l.terminal.snapshot()
			_, err = l.controller.poll(&snapshot, l.writer, l.clock)
		} else {
			l.clock.setElapsed(200 * time.Millisecond)
			stale := l.terminal.snapshot()
			stale.screenSeq = 0
			err = l.controller.observer(l.writer, l.clock).observe(&stale)
		}
		if err == nil || !strings.Contains(cafleetErrorOf(err).message(), "regress") {
			t.Fatalf("%s: clock/sequence corruption remains an explicit invariant error: %v", invariant, err)
		}
		if status := l.controller.status(); status.gapCount != before || status.diagnostic != none[string]() {
			t.Fatalf("%s: status %+v", invariant, status)
		}
		assertOpenEpisode(t, l)
	}
}

func controllerTimestamp(t *testing.T, millis int) string {
	t.Helper()
	origin, ok := parseLenient(observationFixtureNow)
	if !ok {
		t.Fatal("the fixture stamp is canonical")
	}
	return formatUTC(origin.Add(time.Duration(millis) * time.Millisecond))
}

func TestRuntimeObservation_ControllerTests_FailureHandling_HealthReportsNoWriterDiagnosticOrGapStartWhileHealthy(t *testing.T) {
	t.Parallel()
	l := newLiveCase(t, nil)
	health := l.controller.health(l.writer)
	if health.Degraded || health.WriterDiagnostic != none[string]() || health.GapOpenSince != none[string]() {
		t.Fatalf("health = %+v", health)
	}
	if err := health.validate(); err != nil {
		t.Fatal(err)
	}
}

func TestRuntimeObservation_ControllerTests_FailureHandling_HealthReportsTheSanitizedWriterDiagnosticAndThePendingGapStart(t *testing.T) {
	t.Parallel()
	l := newLiveCase(t, nil)
	l.hooks.mu.Lock()
	l.hooks.busy[writerBoundarySlot(writerBoundaryPersist)] = true
	l.hooks.mu.Unlock()
	l.clock.setElapsed(200 * time.Millisecond)
	mustFeed(t, l.terminal, controllerForm("write second.txt"))
	observeSnapshot(t, l.controller, l.writer, l.clock, l.terminal)
	l.clock.setElapsed(300 * time.Millisecond)
	pollStatus(t, l.controller, l.writer, l.clock, l.terminal)
	waitUntilWithin(t, 10*time.Second, func() bool { return l.writer.status().diagnostic != none[string]() })
	l.clock.setElapsed(400 * time.Millisecond)
	openedAt := wallStamp(l.clock)
	pollStatus(t, l.controller, l.writer, l.clock, l.terminal)
	health := l.controller.health(l.writer)
	writerDiagnostic, _ := l.writer.status().diagnostic.get()
	if health.Eligibility != (inputEligibilityHold{reason: gateReasonGap}) || !health.Degraded || health.Diagnostic != some("sink_unavailable") ||
		health.WriterDiagnostic != some(diagnosticText(writerDiagnostic)) || health.GapOpenSince != some(openedAt) {
		t.Fatalf("health = %+v", health)
	}
	if err := health.validate(); err != nil {
		t.Fatal(err)
	}
	l.hooks.releaseForCleanup()
}

func TestRuntimeObservation_ControllerTests_FailureHandling_HealthReportsATerminalFencedGapWithoutAGapStart(t *testing.T) {
	t.Parallel()
	l := newLiveCase(t, nil)
	l.exit(300)
	waitUntilWithin(t, 10*time.Second, func() bool { return committed(t, l.fixture) == 3 && l.writer.status().pendingCount == 0 })
	l.clock.setElapsed(400 * time.Millisecond)
	if err := l.controller.observerFailed(observationFailureParser, l.clock); err != nil {
		t.Fatal(err)
	}
	health := l.controller.health(l.writer)
	if health.Eligibility != (inputEligibilityHold{reason: gateReasonGap}) || !health.Degraded || health.WriterDiagnostic != none[string]() || health.GapOpenSince != none[string]() {
		t.Fatalf("health = %+v", health)
	}
	if err := health.validate(); err != nil {
		t.Fatal(err)
	}
}

func TestRuntimeObservation_ControllerTests_FailureHandling_RunnerFailureAfterTerminalEnqueueNeverAppendsALivePostTerminalGap(t *testing.T) {
	t.Parallel()
	l := newLiveCase(t, nil)
	l.hooks.mu.Lock()
	l.hooks.busy[writerBoundarySlot(writerBoundaryPersist)] = true
	l.hooks.mu.Unlock()
	l.exit(300)
	l.hooks.wait(t, writerBoundaryPersist, 2)
	waitUntilWithin(t, 2*time.Second, func() bool { return l.writer.status().diagnostic != none[string]() })
	l.clock.setElapsed(500 * time.Millisecond)
	pollStatus(t, l.controller, l.writer, l.clock, l.terminal)
	l.clock.setElapsed(700 * time.Millisecond)
	l.hooks.releaseForCleanup()
	pending, unsaved := l.drain(realDeadline())
	events := runEvents(t, l.fixture)
	if pending != 0 || unsaved != 0 || l.controller.hasPending() || l.controller.unsavedGapCount() != 0 || l.controller.status().gapCount != 0 {
		t.Fatalf("drain (%d, %d) pending %v", pending, unsaved, l.controller.hasPending())
	}
	var kinds []runtimeEventKind
	for _, view := range events {
		kinds = append(kinds, view.event.kind)
	}
	if !slices.Equal(kinds, []runtimeEventKind{runtimeEventKindInteractionOpened, runtimeEventKindInteractionClosed, runtimeEventKindRunnerExited}) {
		t.Fatalf("kinds = %v", kinds)
	}
	if last := events[len(events)-1].event.observedAt; last != controllerTimestamp(t, 300) {
		t.Fatalf("observed at %s", last)
	}
	for range 3 {
		pollStatus(t, l.controller, l.writer, l.clock, l.terminal)
	}
	if len(runEvents(t, l.fixture)) != len(events) || l.controller.hasPending() {
		t.Fatal("no live post-terminal gap is appended")
	}
}

func TestRuntimeObservation_ControllerTests_ControllerQuietPollOpensOneEpisodeWithTheUnchangedParserSequence(t *testing.T) {
	t.Parallel()
	f := newWriterFixture(t)
	configureController(t, f)
	writer := startReal(t, f, &writerTestHooks{})
	c := observationFixtureClock(t)
	controller, _, terminal := startedController(t, f, writer, c, syntheticCatalog(t))
	feedObservedBy(t, terminal, controllerForm("write disposable.txt"), controller, writer, c)
	sequence := terminal.snapshot().screenSeq
	if controller.status().eligibility != (inputEligibilityHold{reason: gateReasonCandidate}) {
		t.Fatalf("status %+v", controller.status())
	}
	c.setElapsed(99 * time.Millisecond)
	pollStatus(t, controller, writer, c, terminal)
	if got := committed(t, f); got != 0 {
		t.Fatalf("committed %d", got)
	}
	c.setElapsed(100 * time.Millisecond)
	pollStatus(t, controller, writer, c, terminal)
	yieldUntil(t, func() bool { return committed(t, f) == 1 })
	view := readEvent(t, f.conn, 1)
	interaction, ok := view.interaction.get()
	if !ok || interaction.runID != f.run.runID || interaction.episodeSeq != 1 {
		t.Fatalf("the opening links its persisted episode: %+v", interaction)
	}
	if view.event.kind != runtimeEventKindInteractionOpened || view.event.interactionID != some(interaction.interactionID) {
		t.Fatalf("event %+v", view.event)
	}
	assertJSONEqual(t, jsonAt(t, view.event.payloadJSON, "evidence", "screen_seq"), mustJSON(t, sequence), "screen_seq")
	if terminal.snapshot().screenSeq != sequence {
		t.Fatal("the poll leaves the parser sequence unchanged")
	}
	writer.finish(realDeadline())
}

func TestRuntimeObservation_ControllerTests_ControllerActualAdapterObservesTransientFormBeforeClearWithoutOpening(t *testing.T) {
	t.Parallel()
	f := newWriterFixture(t)
	configureController(t, f)
	writer := startReal(t, f, &writerTestHooks{})
	c := observationFixtureClock(t)
	controller, _, terminal := startedController(t, f, writer, c, syntheticCatalog(t))
	feedObservedBy(t, terminal, append(controllerForm("write disposable.txt"), "\x1b[2J\x1b[Hworking"...), controller, writer, c)
	c.setElapsed(100 * time.Millisecond)
	status := pollStatus(t, controller, writer, c, terminal)
	if _, held := status.eligibility.(inputEligibilityHold); !held || committed(t, f) != 0 || status.retainedTransition || status.retainedDraft {
		t.Fatalf("status %+v", status)
	}
	writer.finish(realDeadline())
}

func TestRuntimeObservation_ControllerTests_ControllerTypedFailuresKeepMetadataOnlyDiagnosticsAndHoldInput(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		reason observationFailure
		code   string
	}{{observationFailureParser, "parser_unavailable"}, {observationFailureDetector, "detector_unavailable"}, {observationFailureSink, "sink_unavailable"}} {
		f := newWriterFixture(t)
		configureController(t, f)
		controller := newController(t, f.run, syntheticCatalog(t))
		if err := controller.observerFailed(tc.reason, observationFixtureClock(t)); err != nil {
			t.Fatal(err)
		}
		status := controller.status()
		if _, held := status.eligibility.(inputEligibilityHold); !held || status.diagnostic != some(tc.code) || status.gapCount == 0 {
			t.Fatalf("%s: status %+v", tc.code, status)
		}
	}
}

func TestRuntimeObservation_ControllerTests_ControllerStartedValidatesCurrentIdentityAndIsSingleUse(t *testing.T) {
	t.Parallel()
	f := newWriterFixture(t)
	configureController(t, f)
	writer := startReal(t, f, &writerTestHooks{})
	c := observationFixtureClock(t)
	controller := newController(t, f.run, syntheticCatalog(t))
	screen := mustTerminalState(t, terminalSize{rows: 24, cols: 80}).snapshot()
	if controller.start(f.run, &screen, writer, c) == nil {
		t.Fatal("a run that is not running is refused")
	}
	child := spawnCat(t)
	markRunning(t, f, child.identity)
	stale := f.run
	stale.ownerEpoch++
	if controller.start(stale, &screen, writer, c) == nil {
		t.Fatal("a stale owner is refused")
	}
	if err := controller.start(f.run, &screen, writer, c); err != nil {
		t.Fatal(err)
	}
	if controller.start(f.run, &screen, writer, c) == nil {
		t.Fatal("the controller starts once")
	}
	writer.finish(realDeadline())
}

func TestRuntimeObservation_ControllerTests_ControllerExitClosesTheOpenEpisodeBeforeRecordingTheRealNativeSignal(t *testing.T) {
	t.Parallel()
	f := newWriterFixture(t)
	configureController(t, f)
	writer := startReal(t, f, &writerTestHooks{})
	c := observationFixtureClock(t)
	controller, child, terminal := startedController(t, f, writer, c, syntheticCatalog(t))
	feedObservedBy(t, terminal, controllerForm("write disposable.txt"), controller, writer, c)
	c.setElapsed(100 * time.Millisecond)
	pollStatus(t, controller, writer, c, terminal)
	waitUntilWithin(t, 20*time.Second, func() bool { return committed(t, f) == 1 })
	if !child.alive(t) {
		t.Fatal("the child runs")
	}
	child.terminate(t)
	c.setElapsed(200 * time.Millisecond)
	snapshot := terminal.snapshot()
	if err := controller.childExited(childExitSignal{signal: 15}, &snapshot, writer, c); err != nil {
		t.Fatal(err)
	}
	waitUntilWithin(t, 20*time.Second, func() bool { return committed(t, f) == 3 })
	if kinds := eventKinds(t, f); !slices.Equal(kinds, []string{"interaction_opened", "interaction_closed", "runner_exited"}) {
		t.Fatalf("kinds = %q", kinds)
	}
	if run := mustRun(t, f.conn, f.run.runID); run.exitCode != none[int64]() {
		t.Fatalf("exit code %v", run.exitCode)
	}
	writer.finish(realDeadline())
}

// pressureDrainBudget bounds draining 64 retained pressure transitions through SQLite, which
// exceeds the shared 2 s budget under a concurrent full suite on slower storage.
const pressureDrainBudget = 10 * time.Second

func assertRetainedPressure(t *testing.T, err error, status observationStatus) {
	t.Helper()
	if err == nil {
		return
	}
	if !status.retainedTransition && !status.retainedDraft && status.gapCount == 0 {
		t.Fatalf("an observer error requires retained work or a recorded gap: %v", err)
	}
	if _, held := status.eligibility.(inputEligibilityHold); !held {
		t.Fatalf("an observer error holds eligibility: %v", err)
	}
}

func TestRuntimeObservation_ControllerTests_ControllerRetainsBothSupersededOriginalsWhenTheFirstSubmitHitsPressure(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	f := newWriterFixture(t)
	configureController(t, f)
	catalog := mustLoadCatalog(t, syntheticDefinitions())
	hooks := &writerTestHooks{}
	hooks.pause(writerBoundaryReceive)
	writer := startReal(t, f, hooks)
	hooks.wait(t, writerBoundaryReceive, 1)
	c := observationFixtureClock(t)
	controller, _, terminal := startedController(t, f, writer, c, catalog)
	preview, err := insertMessageValidated(ctx, f.conn, f.run.directorMemberID, f.run.memberID, "preview")
	if err != nil {
		t.Fatal(err)
	}
	controller.publishDeliveryResult(previewDeliveryResultCause(preview.messageID, deliveryMethodProfile, "partial", stopCauseShortWrite))
	pollStatus(t, controller, writer, c, terminal)
	if pending := writer.status().pendingCount; pending != 1 {
		t.Fatalf("pending %d", pending)
	}
	lastOpeningSequence := uint64(0)
	for index := range uint64(32) {
		c.setElapsed(time.Duration(index*200) * time.Millisecond) //nolint:gosec // index is below 32
		mustFeed(t, terminal, controllerForm(fmt.Sprintf("write fixture-%02d.txt", index)))
		lastOpeningSequence = terminal.snapshot().screenSeq
		observeSnapshot(t, controller, writer, c, terminal)
		c.setElapsed(time.Duration(index*200+100) * time.Millisecond) //nolint:gosec // index is below 32
		pollStatus(t, controller, writer, c, terminal)
		if pending := writer.status().pendingCount; pending != index*2+2 || controller.hasPending() {
			t.Fatalf("index %d: pending %d", index, pending)
		}
	}
	if status := writer.status(); status.pendingCount != 64 || status.diagnostic != none[string]() || committed(t, f) != 0 {
		t.Fatalf("writer status %+v", status)
	}
	c.setElapsed(8600 * time.Millisecond)
	replacement := controllerForm("write final-original.txt")
	feedObservedBy(t, terminal, replacement[:1], controller, writer, c)
	if terminal.snapshot().screenSeq <= lastOpeningSequence {
		t.Fatal("the lone escape advances the screen")
	}
	feedObservedBy(t, terminal, replacement[1:], controller, writer, c)
	c.setElapsed(8700 * time.Millisecond)
	snapshot := terminal.snapshot()
	_, err = controller.poll(&snapshot, writer, c)
	assertRetainedPressure(t, err, controller.status())
	originalSequence := terminal.snapshot().screenSeq
	if status := controller.status(); !status.retainedTransition || !status.retainedDraft || writer.status().pendingCount != 64 {
		t.Fatalf("status %+v", status)
	}
	if _, held := controller.status().eligibility.(inputEligibilityHold); !held {
		t.Fatal("input is held")
	}
	err = terminal.feedObserved([]byte("\x1b[2J\x1b[Hlater output during the gap"), controller.observer(writer, c))
	assertRetainedPressure(t, err, controller.status())
	if status := controller.status(); terminal.snapshot().screenSeq <= originalSequence || !status.retainedTransition || !status.retainedDraft {
		t.Fatalf("status %+v", status)
	}
	hooks.releaseForCleanup()
	pollUntil := func(condition func() bool) {
		until := time.Now().Add(pressureDrainBudget)
		for !condition() {
			if !time.Now().Before(until) {
				t.Fatal("bounded worker condition did not complete")
			}
			snapshot := terminal.snapshot()
			_, err := controller.poll(&snapshot, writer, c)
			assertRetainedPressure(t, err, controller.status())
			runtime.Gosched()
		}
	}
	pollUntil(func() bool { return committed(t, f) >= 66 })
	closed := eventViewAt(t, f, 65)
	opened := eventViewAt(t, f, 66)
	if closed.event.kind != runtimeEventKindInteractionClosed {
		t.Fatalf("closed %+v", closed.event)
	}
	assertJSONEqual(t, jsonAt(t, closed.event.payloadJSON, "close_reason"), jsontext.Value(`"superseded"`), "close_reason")
	for _, tc := range []struct {
		view     runtimeEventView
		sequence int64
		episode  int64
		screen   uint64
	}{{closed, 65, 32, lastOpeningSequence}, {opened, 66, 33, originalSequence}} {
		interaction, ok := tc.view.interaction.get()
		if !ok || tc.view.event.runID != f.run.runID || tc.view.event.eventSeq != tc.sequence || tc.view.event.interactionID != some(interaction.interactionID) || interaction.episodeSeq != tc.episode {
			t.Fatalf("event %d: %+v %+v", tc.sequence, tc.view.event, interaction)
		}
		if tc.view.event.observedAt != wallStamp(c) {
			t.Fatalf("the supersede is observed at the clock's current moment: %s", tc.view.event.observedAt)
		}
		assertJSONEqual(t, jsonAt(t, tc.view.event.payloadJSON, "evidence", "screen_seq"), mustJSON(t, tc.screen), "screen_seq")
	}
	if opened.event.kind != runtimeEventKindInteractionOpened {
		t.Fatalf("opened %+v", opened.event)
	}
	pollUntil(func() bool { return !controller.hasPending() && writer.status().pendingCount == 0 })
	if pending, unsaved := writer.finish(time.Now().Add(pressureDrainBudget)); pending != 0 || unsaved != 0 {
		t.Fatalf("finish (%d, %d)", pending, unsaved)
	}
	if !runtimeEventRecordsEqual(eventViewAt(t, f, 65).event, closed.event) || !runtimeEventRecordsEqual(eventViewAt(t, f, 66).event, opened.event) {
		t.Fatal("the committed events are final")
	}
}

var idleComposer = []byte("\x1b[0m\x1b[2J\x1b[23;1HSynthetic state\r\nState end")

func TestRuntimeObservation_ControllerTests_ControllerGapRecoveryReobservesTheUnchangedScreenAfterPublishingADelivery(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	for _, tc := range []struct {
		name           string
		queuesDelivery bool
	}{{"no queued delivery", false}, {"delivery queued during the gap", true}} {
		f := newWriterFixture(t)
		configureController(t, f)
		if _, err := f.conn.execute("UPDATE member_runs SET coverage='unknown' WHERE run_id=?1", f.run.runID); err != nil {
			t.Fatal(err)
		}
		refreshWriterRun(t, f)
		catalog := composerCatalog(t, none[string](), none[string]())
		hooks := &writerTestHooks{}
		hooks.pause(writerBoundaryPersist)
		writer := startReal(t, f, hooks)
		c := observationFixtureClock(t)
		controller, _, terminal := startedController(t, f, writer, c, catalog)
		waitUntilWithin(t, 20*time.Second, func() bool { return writer.status().diagnostic != none[string]() })
		feedObservedBy(t, terminal, idleComposer, controller, writer, c)
		c.setElapsed(100 * time.Millisecond)
		if status := pollStatus(t, controller, writer, c, terminal); status.eligibility != (inputEligibilityHold{reason: gateReasonGap}) {
			t.Fatalf("%s: a degraded writer holds input behind a gap: %v", tc.name, status.eligibility)
		}
		if tc.queuesDelivery {
			preview, err := insertMessageValidated(ctx, f.conn, f.run.directorMemberID, f.run.memberID, "preview")
			if err != nil {
				t.Fatal(err)
			}
			controller.publishDeliveryResult(previewDeliveryResultCause(preview.messageID, deliveryMethodProfile, "partial", stopCauseShortWrite))
		}
		hooks.releaseForCleanup()
		waitUntilWithin(t, 20*time.Second, func() bool {
			state := writer.status()
			return state.diagnostic == none[string]() && state.pendingCount == 0
		})
		status := controller.status()
		for _, millis := range []int{200, 300, 400} {
			c.setElapsed(time.Duration(millis) * time.Millisecond)
			status = pollStatus(t, controller, writer, c, terminal)
		}
		if status.eligibility != (inputEligibilityComposerEmpty{}) {
			t.Fatalf("%s: the unchanged idle screen is re-observed after the gap closes: %v", tc.name, status.eligibility)
		}
		writer.finish(realDeadline())
	}
}

func TestRuntimeObservation_ControllerTests_PublishDeliveryResultQueuesBlockedRecordsPerBlockerAndDropsAQueuedRepeat(t *testing.T) {
	t.Parallel()
	f := newWriterFixture(t)
	configureController(t, f)
	controller := newController(t, f.run, syntheticCatalog(t))
	blocked := func(blocker int64) jsontext.Value {
		return jsontext.Value(fmt.Sprintf(`{"kind":"input_delivery_stalled","input_id":null,"message_id":null,"blocked_by_input_id":%d,"waiting_since":%q}`, blocker, observationFixtureNow))
	}
	controller.publishDeliveryStalled(blocked(11))
	controller.publishDeliveryStalled(blocked(12))
	controller.publishDeliveryStalled(blocked(11))
	if len(controller.deliveryResults) != 2 {
		t.Fatalf("queued %d", len(controller.deliveryResults))
	}
	assertJSONEqual(t, controller.deliveryResults[0], blocked(11), "the first blocker")
	assertJSONEqual(t, controller.deliveryResults[1], blocked(12), "the second blocker")
}

func TestRuntimeObservation_ControllerTests_ControllerReplacesAnEvidenceInvalidRefusalWithItsRejectionAndOpensNoGap(t *testing.T) {
	t.Parallel()
	f := newWriterFixture(t)
	configureController(t, f)
	writer := startReal(t, f, &writerTestHooks{})
	c := observationFixtureClock(t)
	controller, _, terminal := startedController(t, f, writer, c, syntheticCatalog(t))
	forged := transitionDraft{
		runID:          f.run.runID,
		episodeSeq:     none[int64](),
		classification: none[interactionClass](),
		kind:           runtimeEventKindCoverageUnknown,
		observedAt:     observationFixtureNow,
		evidenceJSON:   jsontext.Value(`{"forged":true}`),
		closeReason:    none[interactionCloseReason](),
		causeJSON:      jsontext.Value(`{"kind":"coverage_unknown"}`),
	}
	if err := controller.retain(controller.update([]transitionDraft{forged})); err != nil {
		t.Fatal(err)
	}
	c.setElapsed(100 * time.Millisecond)
	status := pollStatus(t, controller, writer, c, terminal)
	if status.gapCount != 0 || status.diagnostic != none[string]() || status.retainedTransition || controller.hasPending() {
		t.Fatalf("status %+v", status)
	}
	waitUntilWithin(t, 20*time.Second, func() bool { return committed(t, f) == 1 && writer.status().pendingCount == 0 })
	if state := writer.status(); state.diagnostic != none[string]() || state.gapCount != 0 {
		t.Fatalf("writer %+v", state)
	}
	event := eventViewAt(t, f, 1).event
	if event.kind != runtimeEventKindRunnerDegraded {
		t.Fatalf("event %+v", event)
	}
	cause := jsonAt(t, event.payloadJSON, "cause")
	assertJSONEqual(t, jsonAt(t, cause, "kind"), jsontext.Value(`"event_rejected"`), "kind")
	assertJSONEqual(t, jsonAt(t, cause, "rejected_kind"), jsontext.Value(`"coverage_unknown"`), "rejected_kind")
	assertJSONEqual(t, jsonAt(t, cause, "rejected_observed_at"), mustJSON(t, observationFixtureNow), "rejected_observed_at")
	if decodedString(t, jsonAt(t, cause, "error")) == "" {
		t.Fatalf("cause %s", cause)
	}
	assertJSONEqual(t, jsonAt(t, event.payloadJSON, "evidence", "rule_id"), jsontext.Value(`"runner-lifecycle"`), "rule_id")
	assertJSONEqual(t, jsonAt(t, event.payloadJSON, "evidence", "screen_seq"), mustJSON(t, terminal.snapshot().screenSeq), "screen_seq")
	if controller.status().gapCount != 0 {
		t.Fatal("no gap")
	}
	writer.finish(realDeadline())
}

func TestRuntimeObservation_InspectionTests_PermissionInspectionObservedAuthorityHasNoDebugOrSerializedRepresentation(t *testing.T) {
	t.Parallel()
	request := observedRequest{requestRuleID: "observed-authority-private-rule", choices: []string{"private-choice"}, focus: "private-focus"}
	assertRedacted(t, "observedRequest", request)
	assertRedacted(t, "*observedRequest", &request)
}

// nativeCaptureEnv names the private configuration of a native capture child.
const nativeCaptureEnv = "CAFLEET_NATIVE_CAPTURE_CONFIG"

type nativeCaptureLimits struct {
	maxCells              int
	maxSourceBytes        int
	maxTitleBytes         int
	maxPatterns           int
	maxPatternBytes       int
	maxTotalPatternBytes  int
	maxCompiledRegexBytes int
	maxPatternInputBytes  int
	maxScratchBytes       int
	maxProjectedHeapBytes int
	maxEncodedSampleBytes int
	maxEncodedResultBytes int
	maxArmBytes           int
	maxConfigBytes        int
	maxProducerBytes      int
	maxDiagnosticBytes    int
	maxInterval           time.Duration
	maxRequestDuration    time.Duration
}

func nativeCaptureLimitsDefault() nativeCaptureLimits {
	return nativeCaptureLimits{
		maxCells:              32768,
		maxSourceBytes:        1048576,
		maxTitleBytes:         16384,
		maxPatterns:           64,
		maxPatternBytes:       4096,
		maxTotalPatternBytes:  65536,
		maxCompiledRegexBytes: 262144,
		maxPatternInputBytes:  67108864,
		maxScratchBytes:       2097152,
		maxProjectedHeapBytes: 8388608,
		maxEncodedSampleBytes: 2097152,
		maxEncodedResultBytes: 4194304,
		maxArmBytes:           16384,
		maxConfigBytes:        65536,
		maxProducerBytes:      65536,
		maxDiagnosticBytes:    128,
		maxInterval:           5 * time.Second,
		maxRequestDuration:    30 * time.Second,
	}
}

func (l nativeCaptureLimits) integers() []jsonMember {
	return []jsonMember{
		{"max_cells", l.maxCells},
		{"max_source_bytes", l.maxSourceBytes},
		{"max_title_bytes", l.maxTitleBytes},
		{"max_patterns", l.maxPatterns},
		{"max_pattern_bytes", l.maxPatternBytes},
		{"max_total_pattern_bytes", l.maxTotalPatternBytes},
		{"max_compiled_regex_bytes", l.maxCompiledRegexBytes},
		{"max_pattern_input_bytes", l.maxPatternInputBytes},
		{"max_scratch_bytes", l.maxScratchBytes},
		{"max_projected_heap_bytes", l.maxProjectedHeapBytes},
		{"max_encoded_sample_bytes", l.maxEncodedSampleBytes},
		{"max_encoded_result_bytes", l.maxEncodedResultBytes},
		{"max_arm_bytes", l.maxArmBytes},
		{"max_config_bytes", l.maxConfigBytes},
		{"max_producer_bytes", l.maxProducerBytes},
		{"max_diagnostic_bytes", l.maxDiagnosticBytes},
	}
}

// rustDuration is the serde encoding of a std::time::Duration, which the capture artifact keeps.
func rustDuration(d time.Duration) jsonObject {
	return jsonObject{{"secs", int64(d / time.Second)}, {"nanos", int64(d % time.Second)}}
}

func (l nativeCaptureLimits) MarshalJSONTo(encoder *jsontext.Encoder) error {
	object := jsonObject(l.integers())
	object = append(object, jsonMember{"max_interval", rustDuration(l.maxInterval)}, jsonMember{"max_request_duration", rustDuration(l.maxRequestDuration)})
	return json.MarshalEncode(encoder, object)
}

func (l nativeCaptureLimits) validate() error {
	defaults := nativeCaptureLimitsDefault().integers()
	for index, member := range l.integers() {
		value, ok := member.value.(int)
		if !ok {
			panic("every integer capture limit is an int")
		}
		if err := ensure(value > 0 && value <= defaults[index].value.(int), "capture limit outside approved bounds"); err != nil {
			return err
		}
	}
	return ensure(l.maxInterval >= 100*time.Millisecond && l.maxInterval <= 5*time.Second &&
		l.maxRequestDuration > 100*time.Millisecond && l.maxRequestDuration <= 30*time.Second,
		"capture duration outside bounds")
}

const nativeCaptureSampleCount = 2

type captureScreen string

const (
	captureScreenPrimary   captureScreen = "primary"
	captureScreenAlternate captureScreen = "alternate"
	captureScreenEither    captureScreen = "either"
)

func (s *captureScreen) UnmarshalText(text []byte) error {
	switch selected := captureScreen(text); selected {
	case captureScreenPrimary, captureScreenAlternate, captureScreenEither:
		*s = selected
		return nil
	default:
		return appErrorf("unknown variant `%s`", text)
	}
}

type captureMaskReason string

const captureMaskReasonReviewedPrivacy captureMaskReason = "reviewed_privacy"

func (r *captureMaskReason) UnmarshalText(text []byte) error {
	if captureMaskReason(text) != captureMaskReasonReviewedPrivacy {
		return appErrorf("unknown variant `%s`", text)
	}
	*r = captureMaskReasonReviewedPrivacy
	return nil
}

type captureRegion struct {
	Top    uint16 `json:"top"`
	Left   uint16 `json:"left"`
	Bottom uint16 `json:"bottom"`
	Right  uint16 `json:"right"`
}

type captureMask struct {
	Screen captureScreen     `json:"screen"`
	Rows   uint16            `json:"rows"`
	Cols   uint16            `json:"cols"`
	Region captureRegion     `json:"region"`
	Reason captureMaskReason `json:"reason"`
}

// appliesTo reports whether the mask selects the screen mode the runner snapshot shows.
func (m captureMask) appliesTo(screen *screenSnapshot) bool {
	return m.Screen == captureScreenEither || (m.Screen == captureScreenAlternate) == runnerScreenMode(screen)
}

func captureTriggerName(trigger captureTrigger) string {
	switch trigger {
	case captureTriggerChildOutput:
		return "child_output"
	case captureTriggerResize:
		return "resize"
	case captureTriggerTimer:
		return "timer"
	default:
		panic(fmt.Sprintf("unknown capture trigger %d", trigger))
	}
}

type nativeCaptureStatus string

const (
	nativeCaptureStatusComplete      nativeCaptureStatus = "complete"
	nativeCaptureStatusDeadline      nativeCaptureStatus = "deadline"
	nativeCaptureStatusStaleIdentity nativeCaptureStatus = "stale_identity"
	nativeCaptureStatusCapacity      nativeCaptureStatus = "capacity"
	nativeCaptureStatusOwnerExit     nativeCaptureStatus = "owner_exit"
	nativeCaptureStatusCancelled     nativeCaptureStatus = "cancelled"
)

// boundedCapturePattern compiles a capture pattern; Go's regexp has no compiled-size limit, so
// only the selected limit's bounds are enforced.
func boundedCapturePattern(pattern string, maxCompiledBytes int) (*regexp.Regexp, error) {
	if err := ensure(maxCompiledBytes > 0 && maxCompiledBytes <= nativeCaptureLimitsDefault().maxCompiledRegexBytes, "capture compiled pattern limit outside bounds"); err != nil {
		return nil, err
	}
	compiled, err := regexp.Compile(pattern)
	if err != nil {
		return nil, appError("invalid bounded capture pattern")
	}
	return compiled, nil
}

type nativeCapturePolicy struct {
	metadata  bool
	patterns  []*regexp.Regexp
	sensitive []string
	masks     []captureMask
	catalog   *profileCatalog
}

func (p *nativeCapturePolicy) applyLimits(limits nativeCaptureLimits) error {
	if err := ensure(len(p.patterns) <= limits.maxPatterns, "capture pattern count exceeds selected limit"); err != nil {
		return err
	}
	total := 0
	for _, pattern := range p.patterns {
		total += len(pattern.String())
		if err := ensure(len(pattern.String()) <= limits.maxPatternBytes && total <= limits.maxTotalPatternBytes, "capture pattern exceeds selected limit"); err != nil {
			return err
		}
	}
	for index, pattern := range p.patterns {
		compiled, err := boundedCapturePattern(pattern.String(), limits.maxCompiledRegexBytes)
		if err != nil {
			return err
		}
		p.patterns[index] = compiled
	}
	return nil
}

func nativeCapturePolicyFromLaunch(masks []captureMask, catalog *profileCatalog) (nativeCapturePolicy, error) {
	for _, mask := range masks {
		r := mask.Region
		if err := ensure(mask.Rows > 0 && mask.Cols > 0 && r.Top < r.Bottom && r.Bottom <= mask.Rows && r.Left < r.Right && r.Right <= mask.Cols, "invalid capture mask geometry"); err != nil {
			return nativeCapturePolicy{}, err
		}
	}
	return nativeCapturePolicy{masks: slices.Clone(masks), catalog: catalog}, nil
}

type nativeCaptureRequest struct {
	captureID       string
	runID           int64
	ownerEpoch      int64
	minimumInterval time.Duration
	deadline        time.Time
}

func validateCaptureID(id string) error {
	ascii := true
	for _, r := range id {
		if r > unicode.MaxASCII || unicode.IsControl(r) || r == '/' || r == '\\' {
			ascii = false
		}
	}
	return ensure(strings.TrimSpace(id) != "" && len(id) <= 128 && ascii, "invalid capture identity")
}

// captureMailbox is the state the runner's capture port shares with its receiver.
type captureMailbox struct {
	mu            sync.Mutex
	identity      *controlIdentity
	realClock     bool
	now           time.Time
	used          bool
	closed        bool
	shutdownUntil opt[time.Time]
	request       *nativeCaptureRequest
	result        *nativeCaptureResult
}

// clip bounds until by the shutdown deadline; the caller holds mu.
func (m *captureMailbox) clip(until time.Time) time.Time {
	if end, set := m.shutdownUntil.get(); set {
		return earliestTime(end, until)
	}
	return until
}

type capturePreparedSource struct {
	run   runRecord
	place placement
}

type nativeCapturePort struct {
	seed            runRecord
	serviceProducer *nativeCaptureProducer
	exportStart     func() error
	prepared        *capturePreparedSource
	policy          nativeCapturePolicy
	limits          nativeCaptureLimits
	mailbox         *captureMailbox
	origin          *observationMoment
	source          jsonObject
	producerRef     string
	detector        *detector
	samples         []nativeCaptureSample
	projectedBytes  int
	nextSample      opt[time.Time]
	// titles is the runner's terminalCallbacks; the runner goroutine both feeds and observes.
	titles *terminalTitles
}

type nativeCaptureReceiver struct {
	mailbox *captureMailbox
	limits  nativeCaptureLimits
}

type nativeCaptureResult struct {
	SchemaVersion uint32                `json:"schema_version"`
	CaptureID     string                `json:"capture_id"`
	Source        jsonObject            `json:"source"`
	Policy        jsonObject            `json:"policy"`
	Limits        nativeCaptureLimits   `json:"limits"`
	Status        nativeCaptureStatus   `json:"status"`
	Samples       []nativeCaptureSample `json:"samples"`
	Diagnostic    opt[string]           `json:"diagnostic"`
	producerRef   string
}

type nativeCaptureSample struct {
	SampleIndex        int                 `json:"sample_index"`
	Trigger            string              `json:"trigger"`
	MonotonicStartNS   uint64              `json:"monotonic_start_ns"`
	MonotonicEndNS     uint64              `json:"monotonic_end_ns"`
	ObservedAt         string              `json:"observed_at"`
	ScreenSeq          uint64              `json:"screen_seq"`
	Rows               uint16              `json:"rows"`
	Cols               uint16              `json:"cols"`
	CursorRow          uint16              `json:"cursor_row"`
	CursorCol          uint16              `json:"cursor_col"`
	AlternateScreen    bool                `json:"alternate_screen"`
	Title              opt[string]         `json:"title"`
	Projection         string              `json:"projection"`
	UnavailableRegions []unavailableRegion `json:"unavailable_regions"`
	Grid               *[]projectedRow     `json:"grid"`
}

type projectedRow struct {
	Row                  uint16          `json:"row"`
	RowWrapped           opt[bool]       `json:"row_wrapped"`
	UnavailabilityReason opt[string]     `json:"unavailability_reason"`
	Cells                []projectedCell `json:"cells"`
}

type unavailableRegion struct {
	Top    uint16 `json:"top"`
	Left   uint16 `json:"left"`
	Bottom uint16 `json:"bottom"`
	Right  uint16 `json:"right"`
	Reason string `json:"reason"`
}

type projectedColor vt.Color

func (c projectedColor) MarshalJSONTo(encoder *jsontext.Encoder) error {
	switch c.Kind {
	case vt.ColorDefault:
		return json.MarshalEncode(encoder, jsonObject{{"kind", "default"}})
	case vt.ColorIdx:
		return json.MarshalEncode(encoder, jsonObject{{"kind", "indexed"}, {"index", c.R}})
	case vt.ColorRgb:
		return json.MarshalEncode(encoder, jsonObject{{"kind", "rgb"}, {"r", c.R}, {"g", c.G}, {"b", c.B}})
	default:
		panic(fmt.Sprintf("unknown vt color kind %d", c.Kind))
	}
}

type projectedCell struct {
	Availability     string          `json:"availability"`
	Contents         opt[string]     `json:"contents"`
	HasContents      opt[bool]       `json:"has_contents"`
	Wide             opt[bool]       `json:"wide"`
	WideContinuation opt[bool]       `json:"wide_continuation"`
	Fg               *projectedColor `json:"fg"`
	Bg               *projectedColor `json:"bg"`
	Bold             opt[bool]       `json:"bold"`
	Dim              opt[bool]       `json:"dim"`
	Italic           opt[bool]       `json:"italic"`
	Underline        opt[bool]       `json:"underline"`
	Inverse          opt[bool]       `json:"inverse"`
}

func runnerScreenMode(screen *screenSnapshot) bool {
	alternate, set := screen.alternateScreen.get()
	if !set {
		panic("a runner PTY snapshot carries its screen mode")
	}
	return alternate
}

func sameCaptureRun(left, right runRecord) bool {
	return left.runID == right.runID && jsonEqual(left.databaseIdentityJSON, right.databaseIdentityJSON) &&
		left.launchNonce == right.launchNonce && left.memberID == right.memberID && left.fleetID == right.fleetID &&
		left.directorMemberID == right.directorMemberID && left.backend == right.backend &&
		left.nativeVersion == right.nativeVersion && left.requestedMode == right.requestedMode &&
		left.resolvedMode == right.resolvedMode && left.modeSource == right.modeSource &&
		jsonEqual(left.permissionArgvJSON, right.permissionArgvJSON)
}

// captureClockIsReal reports whether c reads the system clock, as Rust's ClockKind::Real.
func captureClockIsReal(c clock) bool {
	switch c.(type) {
	case systemClock, captureSystemClock:
		return true
	default:
		return false
	}
}

func nativeCaptureChannel(run runRecord, policy nativeCapturePolicy, limits nativeCaptureLimits) (*nativeCapturePort, *nativeCaptureReceiver, error) {
	if err := limits.validate(); err != nil {
		return nil, nil, err
	}
	if err := policy.applyLimits(limits); err != nil {
		return nil, nil, err
	}
	if err := ensure(run.runID > 0 && run.ownerEpoch > 0 && (run.status == runStatusStarting || run.status == runStatusPrepared) &&
		!run.childPID.set && !run.childStartIdentity.set && !run.startedAt.set && !run.endedAt.set && !run.recoveryOriginStatus.set,
		"capture seed requires fresh startup state"); err != nil {
		return nil, nil, err
	}
	mailbox := &captureMailbox{now: time.Now()}
	port := &nativeCapturePort{seed: run, policy: policy, limits: limits, mailbox: mailbox, samples: make([]nativeCaptureSample, 0, nativeCaptureSampleCount), titles: &terminalTitles{}}
	return port, &nativeCaptureReceiver{mailbox: mailbox, limits: limits}, nil
}

func (r *nativeCaptureReceiver) finalizedIdentity() *controlIdentity {
	r.mailbox.mu.Lock()
	defer r.mailbox.mu.Unlock()
	return r.mailbox.identity
}

func (r *nativeCaptureReceiver) request(request nativeCaptureRequest) error {
	m := r.mailbox
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.identity == nil {
		return appError("capture source is not finalized")
	}
	if err := ensure(!m.closed && !m.used, "capture channel is closed or already used"); err != nil {
		return err
	}
	if err := validateCaptureID(request.captureID); err != nil {
		return err
	}
	if err := ensure(request.runID == m.identity.RunID && request.ownerEpoch == m.identity.OwnerEpoch, "capture owner mismatch"); err != nil {
		return err
	}
	now := m.now
	if m.realClock {
		now = time.Now()
	}
	if err := ensure(request.minimumInterval >= 100*time.Millisecond && request.minimumInterval <= r.limits.maxInterval &&
		request.deadline.After(now) && request.deadline.Sub(now) <= r.limits.maxRequestDuration,
		"invalid capture interval/deadline"); err != nil {
		return err
	}
	if until, set := m.shutdownUntil.get(); set {
		if err := ensure(!request.deadline.After(until), "capture request exceeds shutdown deadline"); err != nil {
			return err
		}
	}
	m.used = true
	m.request = &request
	return nil
}

// tryTake leases the completed result, trimming samples to the encoded-size limits.
func (r *nativeCaptureReceiver) tryTake() (opt[*nativeCaptureResult], error) {
	r.mailbox.mu.Lock()
	result := r.mailbox.result
	r.mailbox.result = nil
	r.mailbox.mu.Unlock()
	if result == nil {
		return none[*nativeCaptureResult](), nil
	}
	capacity := func() {
		result.Status = nativeCaptureStatusCapacity
		result.Diagnostic = some("capacity")
	}
	if index := slices.IndexFunc(result.Samples, func(sample nativeCaptureSample) bool {
		_, err := encodedCaptureSize(sample, r.limits.maxEncodedSampleBytes)
		return err != nil
	}); index >= 0 {
		result.Samples = result.Samples[:index]
		capacity()
	}
	for len(result.Samples) > 0 {
		if _, err := encodedCaptureSize(result, r.limits.maxEncodedResultBytes); err == nil {
			break
		}
		result.Samples = result.Samples[:len(result.Samples)-1]
		capacity()
	}
	if _, err := encodedCaptureSize(result, r.limits.maxEncodedResultBytes); err != nil {
		capacity()
	}
	return some(result), nil
}

func (r *nativeCaptureReceiver) publicationDeadline(until time.Time) time.Time {
	r.mailbox.mu.Lock()
	defer r.mailbox.mu.Unlock()
	return r.mailbox.clip(until)
}

func (p *nativeCapturePort) beginShutdown(until time.Time) {
	m := p.mailbox
	m.mu.Lock()
	defer m.mu.Unlock()
	m.closed = true
	until = m.clip(until)
	m.shutdownUntil = some(until)
	if m.request != nil {
		m.request.deadline = earliestTime(m.request.deadline, until)
	}
}

func (p *nativeCapturePort) bindServicePrepared(run runRecord, place placement, spawn spawnSpec, c clock) error {
	producer := p.serviceProducer
	if producer == nil {
		return appError("capture service producer configuration required")
	}
	p.serviceProducer = nil
	return p.bindPrepared(run, place, spawn, *producer, c)
}

func (p *nativeCapturePort) bindPrepared(run runRecord, place placement, spawn spawnSpec, producer nativeCaptureProducer, c clock) error {
	if err := ensure(p.prepared == nil && sameCaptureRun(p.seed, run) && run.status == runStatusPrepared && run.ownerKind == ownerKindRunner &&
		!run.childPID.set && !run.childStartIdentity.set && !run.startedAt.set && !run.endedAt.set && !run.recoveryOriginStatus.set,
		"capture requires validated prepared source"); err != nil {
		return err
	}
	epoch := p.seed.ownerEpoch
	if p.seed.ownerKind == ownerKindCreator {
		if epoch == math.MaxInt64 {
			return appError("capture epoch overflow")
		}
		epoch++
	}
	if err := ensure(run.ownerEpoch == epoch && (p.seed.ownerKind == ownerKindCreator || p.seed.owner().equal(run.owner())), "capture prepared owner mismatch"); err != nil {
		return err
	}
	if err := ensure(place.muxPaneID.set, "capture prepared placement/policy missing"); err != nil {
		return err
	}
	native := jsonIndex(producer.value, "native_executable")
	executable := spawn.argv[0]
	digest, err := sha256FileHex(executable)
	if err != nil {
		return ioError(err.Error())
	}
	if err := ensure(filepath.IsAbs(executable) && jsonIsString(jsonIndex(native, "path"), executable) &&
		jsonIsString(jsonIndex(native, "sha256"), digest) && jsonEqual(jsonIndex(native, "version"), jsonValue(run.nativeVersion)),
		"capture actual native executable mismatch"); err != nil {
		return err
	}
	backend, known := codingAgentCodingAgent(run.backend)
	if !known {
		return appError("capture backend unavailable")
	}
	selection, err := backend.resolvePolicy(run.requestedMode)
	if err != nil {
		return err
	}
	if err := ensure(selection.resolvedMode == run.resolvedMode && selection.modeSource == run.modeSource &&
		jsonEqual(jsonValue(selection.permissionArgv), run.permissionArgvJSON), "capture recorded permission policy mismatch"); err != nil {
		return err
	}
	if err := backend.validateSpawnPolicyArgv(spawn.argv, &selection); err != nil {
		return err
	}
	if err := ensure(jsonIsString(jsonIndex(producer.value, "catalog_origin"), p.policy.catalog.captureOrigin), "capture selected catalog origin mismatch"); err != nil {
		return err
	}
	p.detector = newDetector(p.policy.catalog, backend, run.nativeVersion)
	origin := observationMomentSample(c)
	p.mailbox.mu.Lock()
	p.mailbox.now = origin.monotonic
	p.mailbox.realClock = captureClockIsReal(c)
	p.mailbox.mu.Unlock()
	kind := "deterministic"
	if captureClockIsReal(c) {
		kind = "real"
	}
	p.source = jsonObject{
		{"database_identity_json", run.databaseIdentityJSON},
		{"run_id", run.runID},
		{"owner_epoch", run.ownerEpoch},
		{"launch_nonce", run.launchNonce},
		{"member_id", run.memberID},
		{"fleet_id", run.fleetID},
		{"director_member_id", run.directorMemberID},
		{"runner", jsonObject{{"pid", run.ownerPID}, {"start_identity", run.ownerStartIdentity}}},
		{"child", nil},
		{"backend", run.backend},
		{"native_version", run.nativeVersion},
		{"requested_mode", run.requestedMode},
		{"resolved_mode", run.resolvedMode},
		{"mode_source", run.modeSource.asStr()},
		{"permission_argv", run.permissionArgvJSON},
		{"executable", native},
		{"cwd", spawn.cwd},
		{"environment_evidence_ref", jsonIndex(producer.value, "environment_evidence_ref")},
		{"placement", outputJSONPlacement(place)},
		{"producer", producer.value},
		{"clock", jsonObject{
			{"kind", kind},
			{"monotonic_api", "time.Now monotonic reading"},
			{"utc_api", "time.Now().UTC"},
			{"origin", jsonObject{{"phase", "prepared"}, {"observed_at", origin.observedAt}}},
		}},
	}
	p.producerRef = producer.reference
	p.origin = &origin
	p.prepared = &capturePreparedSource{run: run, place: place}
	return nil
}

func (p *nativeCapturePort) finalizeSource(run runRecord, place placement, child processIdentity) error {
	if p.prepared == nil {
		return appError("capture source is not prepared")
	}
	matches, err := child.matches()
	if err != nil {
		return err
	}
	previous := p.prepared.run
	p.mailbox.mu.Lock()
	if err := ensure(p.mailbox.identity == nil && sameCaptureRun(previous, run) && previous.owner().equal(run.owner()) &&
		reflect.DeepEqual(p.prepared.place, place) && run.status == runStatusRunning && run.startedAt.set &&
		run.childPID == some(child.pid) && run.childStartIdentity == some(child.startIdentity) && matches,
		"capture actual running source mismatch"); err != nil {
		p.mailbox.mu.Unlock()
		return err
	}
	p.source = p.source.with("child", child)
	identity := controlIdentityFromRun(run)
	p.mailbox.identity = &identity
	p.mailbox.mu.Unlock()
	if start := p.exportStart; start != nil {
		p.exportStart = nil
		return start()
	}
	return nil
}

func (p *nativeCapturePort) complete(request nativeCaptureRequest, status nativeCaptureStatus) {
	mode := "redacted"
	if p.policy.metadata {
		mode = "metadata"
	}
	diagnostic := none[string]()
	switch status {
	case nativeCaptureStatusComplete:
	case nativeCaptureStatusDeadline, nativeCaptureStatusCapacity, nativeCaptureStatusOwnerExit, nativeCaptureStatusStaleIdentity:
		diagnostic = some(string(status))
	case nativeCaptureStatusCancelled:
		diagnostic = some("cancelled")
	}
	masks := p.policy.masks
	if masks == nil {
		masks = []captureMask{}
	}
	result := &nativeCaptureResult{
		producerRef:   p.producerRef,
		SchemaVersion: 1,
		CaptureID:     request.captureID,
		Source:        p.source,
		Policy:        jsonObject{{"mode", mode}, {"policy_id", "launch_event_evidence"}, {"masks", masks}},
		Limits:        p.limits,
		Status:        status,
		Samples:       p.samples,
		Diagnostic:    diagnostic,
	}
	p.samples = nil
	p.mailbox.mu.Lock()
	p.mailbox.request = nil
	p.mailbox.result = result
	p.mailbox.mu.Unlock()
	p.nextSample = none[time.Time]()
}

func (p *nativeCapturePort) cancel(reason nativeCaptureStatus) error {
	if err := ensure(reason == nativeCaptureStatusOwnerExit || reason == nativeCaptureStatusCancelled, "invalid external capture cancellation"); err != nil {
		return err
	}
	p.mailbox.mu.Lock()
	p.mailbox.closed = true
	request := p.mailbox.request
	p.mailbox.mu.Unlock()
	if request != nil {
		p.complete(*request, reason)
	}
	return nil
}

func (p *nativeCapturePort) cancelOwnerExit() error {
	return p.cancel(nativeCaptureStatusOwnerExit)
}

func (p *nativeCapturePort) observe(screen *screenSnapshot, trigger captureTrigger, c clock) error {
	p.mailbox.mu.Lock()
	pending := p.mailbox.request
	p.mailbox.mu.Unlock()
	if pending == nil {
		return nil
	}
	request := *pending
	start := observationMomentSample(c)
	p.mailbox.mu.Lock()
	if start.monotonic.Before(p.mailbox.now) {
		p.mailbox.mu.Unlock()
		return appError("capture clock regressed")
	}
	p.mailbox.now = start.monotonic
	p.mailbox.mu.Unlock()
	if !start.monotonic.Before(request.deadline) {
		p.complete(request, nativeCaptureStatusDeadline)
		return nil
	}
	if next, set := p.nextSample.get(); set && start.monotonic.Before(next) {
		return nil
	}
	sample, projected, fits, err := p.project(screen, trigger, start, c, request.deadline)
	if err != nil {
		if cafleetErrorOf(err).kind == errorDeadlineExceeded {
			p.complete(request, nativeCaptureStatusDeadline)
			return nil
		}
		return err
	}
	if !fits {
		p.complete(request, nativeCaptureStatusCapacity)
		return nil
	}
	end := observationMomentSample(c)
	if end.monotonic.Before(start.monotonic) {
		return appError("capture end clock regressed")
	}
	if !end.monotonic.Before(request.deadline) {
		p.complete(request, nativeCaptureStatusDeadline)
		return nil
	}
	p.nextSample = some(end.monotonic.Add(request.minimumInterval))
	p.samples = append(p.samples, sample)
	p.projectedBytes = projected
	if len(p.samples) == nativeCaptureSampleCount {
		p.complete(request, nativeCaptureStatusComplete)
	}
	return nil
}

// screenTitle is the last window title the runner PTY's screen model reported, as Rust's
// test-only ScreenSnapshot title.
func (p *nativeCapturePort) screenTitle() string {
	return lossyUTF8(p.titles.title)
}

func captureDeadline(c clock, until time.Time) error {
	return runtimeCheckDeadline(observationMomentSample(c).monotonic, until)
}

func captureCell(screen *screenSnapshot, row, col uint16) *vt.Cell {
	cell := screen.screen.Cell(row, col)
	if cell == nil {
		panic("a validated capture grid has every in-range cell")
	}
	return cell
}

// project is the sample of screen, with fits false when a capture limit refuses it.
func (p *nativeCapturePort) project(screen *screenSnapshot, trigger captureTrigger, start observationMoment, c clock, deadline time.Time) (nativeCaptureSample, int, bool, error) {
	limits := p.limits
	screenTitle := p.screenTitle()
	cells := int(screen.rows) * int(screen.cols)
	if cells > limits.maxCells || len(screenTitle) > limits.maxTitleBytes {
		return nativeCaptureSample{}, 0, false, nil
	}
	sourceBytes := 0
	for row := range screen.rows {
		if err := captureDeadline(c, deadline); err != nil {
			return nativeCaptureSample{}, 0, false, err
		}
		for col := range screen.cols {
			cell := captureCell(screen, row, col)
			if !cell.IsWideContinuation() {
				sourceBytes += max(len(cell.Contents()), 1)
			}
		}
	}
	rows := int(screen.rows)
	scratch := sourceBytes + rows + rows*int(reflect.TypeFor[captureTextField]().Size()) + cells
	projected := p.projectedBytes + int(reflect.TypeFor[nativeCaptureSample]().Size()) + sourceBytes + len(screenTitle) + len(start.observedAt) +
		cells*int(reflect.TypeFor[projectedCell]().Size()) + rows*(int(reflect.TypeFor[projectedRow]().Size())+int(reflect.TypeFor[unavailableRegion]().Size()))
	if sourceBytes > limits.maxSourceBytes || scratch > limits.maxScratchBytes || projected > limits.maxProjectedHeapBytes {
		return nativeCaptureSample{}, 0, false, nil
	}
	if err := captureDeadline(c, deadline); err != nil {
		return nativeCaptureSample{}, 0, false, err
	}
	observation, err := p.detector.observe(screen)
	if err != nil {
		return nativeCaptureSample{}, 0, false, err
	}
	sensitiveEvidence, _ := jsonBool(jsonIndex(jsonValue(observation.rawEvidence), "sensitive"))
	sensitive := sensitiveEvidence || slices.Contains(p.policy.sensitive, observation.ruleID)
	layout := slices.ContainsFunc(p.policy.masks, func(mask captureMask) bool {
		return mask.appliesTo(screen) && (mask.Rows != screen.rows || mask.Cols != screen.cols)
	})
	metadata := p.policy.metadata || sensitive || layout
	unavailable := []unavailableRegion{}
	var grid *[]projectedRow
	if metadata {
		reason := "metadata_policy"
		if layout {
			reason = "layout_mismatch"
		} else if sensitive {
			reason = "sensitive_form"
		}
		unavailable = append(unavailable, unavailableRegion{Top: 0, Left: 0, Bottom: screen.rows, Right: screen.cols, Reason: reason})
	} else {
		projectedRows, regions, fits, err := p.projectGrid(screen, sourceBytes, c, deadline)
		if err != nil || !fits {
			return nativeCaptureSample{}, 0, fits, err
		}
		unavailable = regions
		grid = &projectedRows
	}
	title := none[string]()
	if !metadata {
		masked := false
		for _, pattern := range p.policy.patterns {
			if err := captureDeadline(c, deadline); err != nil {
				return nativeCaptureSample{}, 0, false, err
			}
			if pattern.MatchString(screenTitle) {
				masked = true
				break
			}
		}
		if !masked {
			title = some(screenTitle)
		}
	}
	end := observationMomentSample(c)
	elapsed := func(at time.Time) (uint64, error) {
		duration := at.Sub(p.origin.monotonic)
		if duration < 0 {
			return 0, appError("capture origin regressed")
		}
		return uint64(duration), nil
	}
	startNS, err := elapsed(start.monotonic)
	if err != nil {
		return nativeCaptureSample{}, 0, false, err
	}
	endNS, err := elapsed(end.monotonic)
	if err != nil {
		return nativeCaptureSample{}, 0, false, err
	}
	cursor, set := screen.cursor.get()
	if !set {
		panic("a runner PTY snapshot carries its cursor")
	}
	projection := "redacted"
	if metadata {
		projection = "metadata"
	}
	sample := nativeCaptureSample{
		SampleIndex:        len(p.samples),
		Trigger:            captureTriggerName(trigger),
		MonotonicStartNS:   startNS,
		MonotonicEndNS:     endNS,
		ObservedAt:         start.observedAt,
		ScreenSeq:          screen.screenSeq,
		Rows:               screen.rows,
		Cols:               screen.cols,
		CursorRow:          cursor[0],
		CursorCol:          cursor[1],
		AlternateScreen:    runnerScreenMode(screen),
		Title:              title,
		Projection:         projection,
		UnavailableRegions: unavailable,
		Grid:               grid,
	}
	if err := captureDeadline(c, deadline); err != nil {
		return nativeCaptureSample{}, 0, false, err
	}
	return sample, projected, true, nil
}

// captureTextField is one unwrapped logical line of the projected text: its byte span and rows.
type captureTextField struct {
	begin, end  int
	top, bottom uint16
}

func (p *nativeCapturePort) projectGrid(screen *screenSnapshot, sourceBytes int, c clock, deadline time.Time) ([]projectedRow, []unavailableRegion, bool, error) {
	limits := p.limits
	var text strings.Builder
	text.Grow(sourceBytes + int(screen.rows))
	fields := make([]captureTextField, 0, screen.rows)
	var top uint16
	beginning := 0
	for row := range screen.rows {
		if err := captureDeadline(c, deadline); err != nil {
			return nil, nil, false, err
		}
		for col := range screen.cols {
			cell := captureCell(screen, row, col)
			if !cell.IsWideContinuation() {
				if cell.HasContents() {
					text.WriteString(cell.Contents())
				} else {
					text.WriteByte(' ')
				}
			}
		}
		if !screen.screen.RowWrapped(row) {
			fields = append(fields, captureTextField{begin: beginning, end: text.Len(), top: top, bottom: row + 1})
			text.WriteByte('\n')
			beginning = text.Len()
			top = row + 1
		}
	}
	if top < screen.rows {
		fields = append(fields, captureTextField{begin: beginning, end: text.Len(), top: top, bottom: screen.rows})
	}
	projectedText := text.String()
	if len(projectedText) > limits.maxSourceBytes || (len(projectedText)+len(p.screenTitle()))*len(p.policy.patterns) > limits.maxPatternInputBytes {
		return nil, nil, false, nil
	}
	cols := int(screen.cols)
	masked := make([]bool, int(screen.rows)*cols)
	for _, pattern := range p.policy.patterns {
		if err := captureDeadline(c, deadline); err != nil {
			return nil, nil, false, err
		}
		for _, found := range pattern.FindAllStringIndex(projectedText, -1) {
			if err := captureDeadline(c, deadline); err != nil {
				return nil, nil, false, err
			}
			for _, field := range fields {
				if found[0] <= field.end && found[1] > field.begin {
					for row := field.top; row < field.bottom; row++ {
						for col := range cols {
							masked[int(row)*cols+col] = true
						}
					}
				}
			}
		}
	}
	for _, mask := range p.policy.masks {
		if err := captureDeadline(c, deadline); err != nil {
			return nil, nil, false, err
		}
		if mask.appliesTo(screen) {
			for row := mask.Region.Top; row < mask.Region.Bottom; row++ {
				for col := mask.Region.Left; col < mask.Region.Right; col++ {
					masked[int(row)*cols+int(col)] = true
				}
			}
		}
	}
	for row := range screen.rows {
		for col := range screen.cols {
			index := int(row)*cols + int(col)
			if !masked[index] {
				continue
			}
			cell := captureCell(screen, row, col)
			if cell.IsWide() && col+1 < screen.cols {
				masked[index+1] = true
			}
			if cell.IsWideContinuation() && col > 0 {
				masked[index-1] = true
			}
		}
	}
	unavailable := []unavailableRegion{}
	projectedRows := make([]projectedRow, 0, screen.rows)
	for row := range screen.rows {
		if err := captureDeadline(c, deadline); err != nil {
			return nil, nil, false, err
		}
		cells := make([]projectedCell, 0, cols)
		complete := true
		for col := range screen.cols {
			if masked[int(row)*cols+int(col)] {
				complete = false
				cells = append(cells, projectedCell{Availability: "masked"})
				continue
			}
			cell := captureCell(screen, row, col)
			fg, bg := projectedColor(cell.FgColor()), projectedColor(cell.BgColor())
			cells = append(cells, projectedCell{
				Availability:     "available",
				Contents:         some(cell.Contents()),
				HasContents:      some(cell.HasContents()),
				Wide:             some(cell.IsWide()),
				WideContinuation: some(cell.IsWideContinuation()),
				Fg:               &fg,
				Bg:               &bg,
				Bold:             some(cell.Bold()),
				Dim:              some(cell.Dim()),
				Italic:           some(cell.Italic()),
				Underline:        some(cell.Underline()),
				Inverse:          some(cell.Inverse()),
			})
		}
		projected := projectedRow{Row: row, Cells: cells}
		if complete {
			projected.RowWrapped = some(screen.screen.RowWrapped(row))
		} else {
			unavailable = append(unavailable, unavailableRegion{Top: row, Left: 0, Bottom: row + 1, Right: screen.cols, Reason: "masked_cells"})
			projected.UnavailabilityReason = some("masked_cells")
		}
		projectedRows = append(projectedRows, projected)
	}
	return projectedRows, unavailable, true, nil
}

// boundedCaptureWriter fails once more than limit bytes are written, keeping them when buffered.
type boundedCaptureWriter struct {
	written int
	limit   int
	buffer  *bytes.Buffer
}

func (w *boundedCaptureWriter) Write(data []byte) (int, error) {
	if w.written+len(data) > w.limit {
		return 0, errors.New("encoded capture capacity")
	}
	if w.buffer != nil {
		w.buffer.Write(data)
	}
	w.written += len(data)
	return len(data), nil
}

func boundedCaptureJSON(value any, limit int, buffer *bytes.Buffer) (int, error) {
	writer := &boundedCaptureWriter{limit: limit, buffer: buffer}
	if err := json.MarshalWrite(writer, value, json.Deterministic(true)); err != nil {
		return 0, ioError(err.Error())
	}
	return writer.written, nil
}

func encodedCaptureSize(value any, limit int) (int, error) {
	return boundedCaptureJSON(value, limit, nil)
}

func encodeCaptureBounded(value any, limit int) ([]byte, error) {
	var buffer bytes.Buffer
	if _, err := boundedCaptureJSON(value, limit, &buffer); err != nil {
		return nil, err
	}
	return buffer.Bytes(), nil
}

var (
	libtestDigestOnce  sync.Once
	libtestDigestValue string
)

// libtestDigest is the running test binary's digest; the binary does not change during its
// process, so one read serves every producer manifest.
func libtestDigest() string {
	libtestDigestOnce.Do(func() {
		executable, err := os.Executable()
		if err != nil {
			panic(err)
		}
		digest, err := sha256FileHex(executable)
		if err != nil {
			panic(err)
		}
		libtestDigestValue = digest
	})
	return libtestDigestValue
}

// captureFileIdentity is a file's device and inode.
type captureFileIdentity struct{ dev, ino uint64 }

func captureFileIdentityOf(info os.FileInfo) captureFileIdentity {
	stat := fileStat(info)
	return captureFileIdentity{dev: uint64(stat.Dev), ino: stat.Ino} //nolint:gosec,unconvert // Dev is int32 on darwin
}

func capturePrivateDirectory(path string) (captureFileIdentity, error) {
	if err := ensure(filepath.IsAbs(path), "private directory must be absolute"); err != nil {
		return captureFileIdentity{}, err
	}
	info, err := privateMetadata(path, uint32(os.Geteuid()), 0o700, true) //nolint:gosec // uids are non-negative
	if err != nil {
		return captureFileIdentity{}, err
	}
	return captureFileIdentityOf(info), nil
}

func captureReadPrivate(path string, limit int) ([]byte, error) {
	if err := ensure(filepath.IsAbs(path), "private file path must be absolute"); err != nil {
		return nil, err
	}
	if _, err := privateMetadata(path, uint32(os.Geteuid()), 0o600, false); err != nil { //nolint:gosec // uids are non-negative
		return nil, err
	}
	file, err := os.OpenFile(path, os.O_RDONLY|syscall.O_NOFOLLOW, 0)
	if err != nil {
		return nil, ioError(ioErrorText(err))
	}
	defer func() { _ = file.Close() }()
	opened, err := file.Stat()
	if err != nil {
		return nil, ioError(ioErrorText(err))
	}
	named, err := os.Lstat(path)
	if err != nil {
		return nil, ioError(ioErrorText(err))
	}
	if err := ensure(captureFileIdentityOf(opened) == captureFileIdentityOf(named) && opened.Size() <= int64(limit), "private file identity/size mismatch"); err != nil {
		return nil, err
	}
	data, err := io.ReadAll(io.LimitReader(file, int64(limit)+1))
	if err != nil {
		return nil, ioError(ioErrorText(err))
	}
	if err := ensure(len(data) <= limit, "private file exceeds budget"); err != nil {
		return nil, err
	}
	return data, nil
}

func captureReference(value jsontext.Value) (string, error) {
	text, ok := jsonString(value)
	if !ok {
		return "", appError("evidence reference required")
	}
	if err := ensure(strings.TrimSpace(text) != "" && len(text) <= 2048 && !strings.ContainsFunc(text, unicode.IsControl), "invalid evidence reference"); err != nil {
		return "", err
	}
	return text, nil
}

func captureLowerDigest(value jsontext.Value) (string, error) {
	text, ok := jsonString(value)
	if !ok {
		return "", appError("digest string required")
	}
	lower := len(text) == 64 && !strings.ContainsFunc(text, func(r rune) bool { return (r < '0' || r > '9') && (r < 'a' || r > 'f') })
	if err := ensure(lower, "lowercase digest required"); err != nil {
		return "", err
	}
	return text, nil
}

func captureExactKeys(value jsontext.Value, keys ...string) error {
	members, ok := jsonMembers(value)
	if !ok {
		return appError("capture object required")
	}
	present := map[string]bool{}
	for _, member := range members {
		present[member.key] = true
	}
	return ensure(len(members) == len(keys) && !slices.ContainsFunc(keys, func(key string) bool { return !present[key] }), "capture object fields mismatch")
}

type nativeCaptureConfig struct {
	SchemaVersion        uint32 `json:"schema_version"`
	DatabaseURL          string `json:"database_url"`
	DescriptorPath       string `json:"descriptor_path"`
	ExpectedRunID        int64  `json:"expected_run_id"`
	ExpectedLaunchNonce  string `json:"expected_launch_nonce"`
	ArmPath              string `json:"arm_path"`
	OutputDirectory      string `json:"output_directory"`
	ProducerManifestPath string `json:"producer_manifest_path"`
	CapturePolicyPath    string `json:"capture_policy_path"`
}

func readNativeCaptureConfig(path string, limits nativeCaptureLimits) (nativeCaptureConfig, error) {
	if err := limits.validate(); err != nil {
		return nativeCaptureConfig{}, err
	}
	data, err := captureReadPrivate(path, limits.maxConfigBytes)
	if err != nil {
		return nativeCaptureConfig{}, err
	}
	var config nativeCaptureConfig
	if err := decodeStrict(data, &config); err != nil {
		return nativeCaptureConfig{}, ioError(err.Error())
	}
	if err := ensure(config.SchemaVersion == 1 && config.ExpectedRunID > 0, "invalid capture config schema/run"); err != nil {
		return nativeCaptureConfig{}, err
	}
	if _, err := captureLowerDigest(jsonValue(config.ExpectedLaunchNonce)); err != nil {
		return nativeCaptureConfig{}, err
	}
	if _, err := capturePrivateDirectory(config.OutputDirectory); err != nil {
		return nativeCaptureConfig{}, err
	}
	if _, err := capturePrivateDirectory(filepath.Dir(config.ArmPath)); err != nil {
		return nativeCaptureConfig{}, err
	}
	if err := ensure(filepath.IsAbs(config.ArmPath), "absolute arm path required"); err != nil {
		return nativeCaptureConfig{}, err
	}
	for _, evidence := range []string{config.DescriptorPath, config.ProducerManifestPath, config.CapturePolicyPath} {
		if _, err := privateMetadata(evidence, uint32(os.Geteuid()), 0o600, false); err != nil { //nolint:gosec // uids are non-negative
			return nativeCaptureConfig{}, err
		}
		if err := ensure(filepath.IsAbs(evidence), "absolute capture evidence path required"); err != nil {
			return nativeCaptureConfig{}, err
		}
	}
	database, found := strings.CutPrefix(config.DatabaseURL, "sqlite:///")
	if !found {
		return nativeCaptureConfig{}, appError("capture database URL required")
	}
	info, err := os.Stat(database)
	if err := ensure(filepath.IsAbs(database) && err == nil && info.Mode().IsRegular(), "capture database file required"); err != nil {
		return nativeCaptureConfig{}, err
	}
	return config, nil
}

type producerNativeExecutable struct {
	Path               string         `json:"path"`
	Sha256             string         `json:"sha256"`
	Version            jsontext.Value `json:"version" cafleet:"default"`
	VersionEvidenceRef string         `json:"version_evidence_ref"`
}

type producerDocument struct {
	SchemaVersion           uint32                   `json:"schema_version"`
	Kind                    string                   `json:"kind"`
	SourceManifestRef       string                   `json:"source_manifest_ref"`
	SourceManifestSha256    string                   `json:"source_manifest_sha256"`
	LibtestExecutablePath   string                   `json:"libtest_executable_path"`
	LibtestExecutableSha256 string                   `json:"libtest_executable_sha256"`
	ParserVersion           string                   `json:"parser_version"`
	CollectorVersion        string                   `json:"collector_version"`
	CatalogOrigin           string                   `json:"catalog_origin"`
	NativeExecutable        producerNativeExecutable `json:"native_executable"`
	LaunchEvidenceRef       string                   `json:"launch_evidence_ref"`
	EnvironmentEvidenceRef  string                   `json:"environment_evidence_ref"`
}

type nativeCaptureProducer struct {
	value     jsontext.Value
	reference string
}

func readNativeCaptureProducer(path string, limits nativeCaptureLimits) (nativeCaptureProducer, error) {
	data, err := captureReadPrivate(path, limits.maxProducerBytes)
	if err != nil {
		return nativeCaptureProducer{}, err
	}
	var document producerDocument
	if err := decodeStrict(data, &document); err != nil {
		return nativeCaptureProducer{}, ioError(err.Error())
	}
	if document.NativeExecutable.Version == nil {
		document.NativeExecutable.Version = jsonNull
	}
	value := jsonValue(document)
	if err := captureExactKeys(value, "schema_version", "kind", "source_manifest_ref", "source_manifest_sha256", "libtest_executable_path",
		"libtest_executable_sha256", "parser_version", "collector_version", "catalog_origin", "native_executable", "launch_evidence_ref", "environment_evidence_ref"); err != nil {
		return nativeCaptureProducer{}, err
	}
	if err := ensure(document.SchemaVersion == 1 && (document.Kind == "synthetic" || document.Kind == "verification_libtest") &&
		(document.CatalogOrigin == "synthetic" || document.CatalogOrigin == "bundled_unknown") &&
		document.ParserVersion == "0.18.1" && document.CollectorVersion == "1", "invalid capture producer identity"); err != nil {
		return nativeCaptureProducer{}, err
	}
	for _, key := range []string{"source_manifest_ref", "launch_evidence_ref", "environment_evidence_ref"} {
		if _, err := captureReference(jsonIndex(value, key)); err != nil {
			return nativeCaptureProducer{}, err
		}
	}
	sourceDigest, err := captureLowerDigest(jsonIndex(value, "source_manifest_sha256"))
	if err != nil {
		return nativeCaptureProducer{}, err
	}
	if err := captureLibtestProvenance(value); err != nil {
		return nativeCaptureProducer{}, err
	}
	native := jsonIndex(value, "native_executable")
	if err := captureExactKeys(native, "path", "sha256", "version", "version_evidence_ref"); err != nil {
		return nativeCaptureProducer{}, err
	}
	nativePath, err := captureReference(jsonIndex(native, "path"))
	if err != nil {
		return nativeCaptureProducer{}, err
	}
	if err := ensure(filepath.IsAbs(nativePath), "absolute native executable required"); err != nil {
		return nativeCaptureProducer{}, err
	}
	if _, err := captureLowerDigest(jsonIndex(native, "sha256")); err != nil {
		return nativeCaptureProducer{}, err
	}
	if _, err := captureReference(jsonIndex(native, "version_evidence_ref")); err != nil {
		return nativeCaptureProducer{}, err
	}
	if !jsonIsNull(jsonIndex(native, "version")) {
		if _, err := captureReference(jsonIndex(native, "version")); err != nil {
			return nativeCaptureProducer{}, err
		}
	}
	if filepath.IsAbs(document.SourceManifestRef) {
		digest, err := sha256FileHex(document.SourceManifestRef)
		if err != nil {
			return nativeCaptureProducer{}, ioError(err.Error())
		}
		if err := ensure(digest == sourceDigest, "capture source manifest digest mismatch"); err != nil {
			return nativeCaptureProducer{}, err
		}
	}
	return nativeCaptureProducer{value: value, reference: path}, nil
}

func captureLibtestProvenance(value jsontext.Value) error {
	selected, err := captureReference(jsonIndex(value, "libtest_executable_path"))
	if err != nil {
		return err
	}
	current, err := os.Executable()
	if err != nil {
		return ioError(err.Error())
	}
	canonicalCurrent, err := filepath.EvalSymlinks(current)
	if err != nil {
		return ioError(err.Error())
	}
	canonicalSelected, err := filepath.EvalSymlinks(selected)
	if err != nil {
		return ioError(err.Error())
	}
	digest, err := captureLowerDigest(jsonIndex(value, "libtest_executable_sha256"))
	if err != nil {
		return err
	}
	return ensure(filepath.IsAbs(selected) && canonicalSelected == canonicalCurrent && digest == libtestDigest(), "capture libtest provenance mismatch")
}

type nativeCaptureArm struct {
	SchemaVersion     uint32 `json:"schema_version"`
	CaptureID         string `json:"capture_id"`
	RunID             int64  `json:"run_id"`
	OwnerEpoch        int64  `json:"owner_epoch"`
	MinimumIntervalMS uint64 `json:"minimum_interval_ms"`
	TimeoutMS         uint64 `json:"timeout_ms"`
}

// nativeCaptureArmFor is a schema-1 arm for identity with the standard 100 ms minimum interval.
func nativeCaptureArmFor(captureID string, identity controlIdentity, timeoutMS uint64) nativeCaptureArm {
	return nativeCaptureArm{SchemaVersion: 1, CaptureID: captureID, RunID: identity.RunID, OwnerEpoch: identity.OwnerEpoch, MinimumIntervalMS: 100, TimeoutMS: timeoutMS}
}

func (a nativeCaptureArm) toRequest(identity controlIdentity, now time.Time, limits nativeCaptureLimits) (nativeCaptureRequest, error) {
	if err := limits.validate(); err != nil {
		return nativeCaptureRequest{}, err
	}
	if err := validateCaptureID(a.CaptureID); err != nil {
		return nativeCaptureRequest{}, err
	}
	maxMillis := func(d time.Duration) uint64 { return uint64(d / time.Millisecond) } //nolint:gosec // capture limits are positive
	if err := ensure(a.SchemaVersion == 1 && a.RunID > 0 && a.OwnerEpoch > 0 && a.RunID == identity.RunID && a.OwnerEpoch == identity.OwnerEpoch &&
		a.MinimumIntervalMS >= 100 && a.MinimumIntervalMS <= maxMillis(limits.maxInterval) && a.TimeoutMS > a.MinimumIntervalMS &&
		a.TimeoutMS <= maxMillis(limits.maxRequestDuration), "invalid one-shot capture arm"); err != nil {
		return nativeCaptureRequest{}, err
	}
	return nativeCaptureRequest{
		captureID:       a.CaptureID,
		runID:           a.RunID,
		ownerEpoch:      a.OwnerEpoch,
		minimumInterval: time.Duration(a.MinimumIntervalMS) * time.Millisecond,  //nolint:gosec // bounded by maxInterval
		deadline:        now.Add(time.Duration(a.TimeoutMS) * time.Millisecond), //nolint:gosec // bounded by maxRequestDuration
	}, nil
}

type nativeCaptureArmReader struct {
	path     string
	parent   captureFileIdentity
	identity controlIdentity
	limits   nativeCaptureLimits
	used     bool
}

func newNativeCaptureArmReader(path string, identity controlIdentity, limits nativeCaptureLimits) (*nativeCaptureArmReader, error) {
	if err := limits.validate(); err != nil {
		return nil, err
	}
	if err := ensure(filepath.IsAbs(path), "absolute arm path required"); err != nil {
		return nil, err
	}
	parent, err := capturePrivateDirectory(filepath.Dir(path))
	if err != nil {
		return nil, err
	}
	return &nativeCaptureArmReader{path: path, parent: parent, identity: identity, limits: limits}, nil
}

// poll claims the one-shot arm once it appears: none until then, and an error after a claim.
func (r *nativeCaptureArmReader) poll(now time.Time) (opt[nativeCaptureRequest], error) {
	parent, err := capturePrivateDirectory(filepath.Dir(r.path))
	if err != nil {
		return none[nativeCaptureRequest](), err
	}
	if err := ensure(parent == r.parent, "arm parent replaced"); err != nil {
		return none[nativeCaptureRequest](), err
	}
	if err := ensure(!r.used, "one-shot arm already consumed"); err != nil {
		return none[nativeCaptureRequest](), err
	}
	meta, err := os.Lstat(r.path)
	if errors.Is(err, fs.ErrNotExist) {
		return none[nativeCaptureRequest](), nil
	}
	if err != nil {
		return none[nativeCaptureRequest](), ioError(ioErrorText(err))
	}
	r.used = true
	if _, err := privateMetadata(r.path, uint32(os.Geteuid()), 0o600, false); err != nil { //nolint:gosec // uids are non-negative
		return none[nativeCaptureRequest](), err
	}
	claimed := strings.TrimSuffix(r.path, filepath.Ext(r.path)) + ".claimed"
	if err := os.Link(r.path, claimed); err != nil {
		return none[nativeCaptureRequest](), ioError(ioErrorText(err))
	}
	owned, err := os.Lstat(claimed)
	if err != nil {
		return none[nativeCaptureRequest](), ioError(ioErrorText(err))
	}
	if err := ensure(captureFileIdentityOf(owned) == captureFileIdentityOf(meta), "arm changed during claim"); err != nil {
		return none[nativeCaptureRequest](), err
	}
	current, err := os.Lstat(r.path)
	if err != nil {
		return none[nativeCaptureRequest](), ioError(ioErrorText(err))
	}
	if err := ensure(captureFileIdentityOf(current) == captureFileIdentityOf(owned), "arm pathname replaced"); err != nil {
		return none[nativeCaptureRequest](), err
	}
	if err := os.Remove(r.path); err != nil {
		return none[nativeCaptureRequest](), ioError(ioErrorText(err))
	}
	data, err := captureReadPrivate(claimed, r.limits.maxArmBytes)
	if err != nil {
		return none[nativeCaptureRequest](), err
	}
	var arm nativeCaptureArm
	if err := decodeStrict(data, &arm); err != nil {
		return none[nativeCaptureRequest](), ioError(err.Error())
	}
	request, err := arm.toRequest(r.identity, now, r.limits)
	if err != nil {
		return none[nativeCaptureRequest](), err
	}
	return some(request), nil
}

type nativeCaptureExportBoundary uint8

const (
	nativeCaptureExportEncode nativeCaptureExportBoundary = iota + 1
	nativeCaptureExportArtifactPublish
	nativeCaptureExportManifestPublish
)

// name is the Rust Debug name the export hooks record in their marker paths.
func (b nativeCaptureExportBoundary) name() string {
	switch b {
	case nativeCaptureExportEncode:
		return "Encode"
	case nativeCaptureExportArtifactPublish:
		return "ArtifactPublish"
	case nativeCaptureExportManifestPublish:
		return "ManifestPublish"
	default:
		panic(fmt.Sprintf("unknown capture export boundary %d", b))
	}
}

type nativeCaptureExportHooks interface {
	before(boundary nativeCaptureExportBoundary, deadline time.Time) error
	serviceReturned(storageDeadline opt[time.Time], status int32, err error)
}

// captureExportHookReferences is export hooks that count their holders, as Rust's Arc drop: the
// export goroutine retains the hooks while it runs.
type captureExportHookReferences interface {
	retain()
	release()
}

type inertCaptureExportHooks struct{}

func (inertCaptureExportHooks) before(nativeCaptureExportBoundary, time.Time) error { return nil }

func (inertCaptureExportHooks) serviceReturned(opt[time.Time], int32, error) {}

// captureExportControl is the publication authority the exporter shares with its task: cancel
// revokes every later permit, and acknowledge records a publication completed before it.
type captureExportControl struct {
	mailbox      *captureMailbox
	mu           sync.Mutex
	revoked      bool
	acknowledged bool
	active       int
}

func (c *captureExportControl) deadline(requested time.Time) time.Time {
	c.mailbox.mu.Lock()
	defer c.mailbox.mu.Unlock()
	return c.mailbox.clip(requested)
}

// acquire is a permit for one publication operation; the caller calls the returned release.
func (c *captureExportControl) acquire(requested time.Time) (func(), error) {
	until := c.deadline(requested)
	c.mu.Lock()
	defer c.mu.Unlock()
	if err := ensure(!c.revoked, "capture exporter unfinished after publication cancellation"); err != nil {
		return nil, err
	}
	if err := runtimeCheckDeadline(time.Now(), until); err != nil {
		return nil, err
	}
	c.active++
	return func() {
		c.mu.Lock()
		defer c.mu.Unlock()
		if c.active == 0 {
			panic("a released publication permit is owned")
		}
		c.active--
	}, nil
}

func (c *captureExportControl) cancel() opt[time.Time] {
	c.mailbox.mu.Lock()
	until := c.mailbox.shutdownUntil
	c.mailbox.mu.Unlock()
	c.mu.Lock()
	c.revoked = true
	c.mu.Unlock()
	return until
}

func (c *captureExportControl) cancelled() bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.revoked
}

func (c *captureExportControl) acknowledge() error {
	c.mu.Lock()
	defer c.mu.Unlock()
	if err := ensure(!c.revoked, "capture exporter unfinished: publication completed after cancellation"); err != nil {
		return err
	}
	c.acknowledged = true
	return nil
}

// captureOperationPermit is a permit from control, or a plain deadline check without one.
func captureOperationPermit(control *captureExportControl, until time.Time) (func(), error) {
	if control == nil {
		return func() {}, runtimeCheckDeadline(time.Now(), until)
	}
	return control.acquire(until)
}

func capturePublicationPermit(control *captureExportControl, hooks nativeCaptureExportHooks, boundary nativeCaptureExportBoundary, until time.Time) (func(), error) {
	current := until
	if control != nil {
		current = control.deadline(until)
	}
	if err := hooks.before(boundary, current); err != nil {
		return nil, err
	}
	return captureOperationPermit(control, until)
}

// captureExporter runs the export loop on its own goroutine once the capture source is final.
type captureExporter struct {
	control *captureExportControl
	hooks   nativeCaptureExportHooks
	mu      sync.Mutex
	done    chan struct{}
	result  error
}

func installCaptureExporter(port *nativeCapturePort, receiver *nativeCaptureReceiver, config nativeCaptureConfig, limits nativeCaptureLimits, hooks nativeCaptureExportHooks) *captureExporter {
	exporter := &captureExporter{control: &captureExportControl{mailbox: receiver.mailbox}, hooks: hooks}
	port.exportStart = func() error {
		exporter.mu.Lock()
		defer exporter.mu.Unlock()
		if err := ensure(exporter.done == nil, "capture exporter starts once"); err != nil {
			return err
		}
		done := make(chan struct{})
		exporter.done = done
		references, counted := hooks.(captureExportHookReferences)
		if counted {
			references.retain()
		}
		go func() {
			defer close(done)
			if counted {
				defer references.release()
			}
			exporter.result = captureExportLoop(receiver, config, limits, exporter.control, hooks)
		}()
		return nil
	}
	return exporter
}

func (e *captureExporter) finish(status int32, serviceErr error) error {
	until := e.control.cancel()
	e.hooks.serviceReturned(until, status, serviceErr)
	e.mu.Lock()
	done := e.done
	e.mu.Unlock()
	if done == nil {
		return nil
	}
	finished := func() bool {
		select {
		case <-done:
			return true
		default:
			return false
		}
	}
	if end, set := until.get(); set {
		for !finished() {
			remaining := time.Until(end)
			if remaining <= 0 {
				break
			}
			time.Sleep(min(2*time.Millisecond, remaining))
		}
	}
	if !finished() {
		e.control.mu.Lock()
		acknowledged := e.control.acknowledged
		e.control.mu.Unlock()
		return appErrorf("capture exporter unfinished; acknowledged publication=%t; an in-flight operation may outlive helper return", acknowledged)
	}
	return e.result
}

func captureExportLoop(receiver *nativeCaptureReceiver, config nativeCaptureConfig, limits nativeCaptureLimits, control *captureExportControl, hooks nativeCaptureExportHooks) error {
	identity := receiver.finalizedIdentity()
	if identity == nil {
		panic("the exporter starts after its source is finalized")
	}
	reader, err := newNativeCaptureArmReader(config.ArmPath, *identity, limits)
	if err != nil {
		return err
	}
	accepted := none[time.Time]()
	published := false
	for {
		if control.cancelled() {
			return nil
		}
		if until, set := accepted.get(); set {
			if !published {
				leased, err := receiver.tryTake()
				if err != nil {
					return err
				}
				if result, set := leased.get(); set {
					if err := publishNativeCapture(config.OutputDirectory, result, limits, receiver.publicationDeadline(until), control, hooks); err != nil {
						return err
					}
					published = true
				}
			} else if _, err := os.Lstat(config.ArmPath); err == nil {
				return appError("one-shot capture arm replaced after completion")
			} else if !errors.Is(err, fs.ErrNotExist) {
				return ioError(ioErrorText(err))
			}
		} else {
			request, err := reader.poll(time.Now())
			if err != nil {
				return err
			}
			if selected, set := request.get(); set {
				if err := receiver.request(selected); err != nil {
					return err
				}
				accepted = some(selected.deadline)
			}
		}
		time.Sleep(5 * time.Millisecond)
	}
}

// publicationTemporary is a publisher's provisional file; cleanup removes it only while it still
// names the file the publisher created.
type publicationTemporary struct {
	path     string
	file     *os.File
	identity captureFileIdentity
	active   bool
}

func createPublicationTemporary(path string) (*publicationTemporary, error) {
	provisional := strings.TrimSuffix(path, filepath.Ext(path)) + ".provisional"
	file, err := os.OpenFile(provisional, os.O_WRONLY|os.O_CREATE|os.O_EXCL|syscall.O_NOFOLLOW, 0o600)
	if err != nil {
		return nil, ioError(ioErrorText(err))
	}
	info, err := file.Stat()
	if err != nil {
		_ = file.Close()
		return nil, ioError(ioErrorText(err))
	}
	return &publicationTemporary{path: provisional, file: file, identity: captureFileIdentityOf(info), active: true}, nil
}

func (p *publicationTemporary) validate() error {
	info, err := os.Lstat(p.path)
	if err != nil {
		return ioError(ioErrorText(err))
	}
	return ensure(info.Mode().IsRegular() && captureFileIdentityOf(info) == p.identity, "capture temporary identity changed")
}

func (p *publicationTemporary) cleanup() error {
	if !p.active {
		return nil
	}
	p.active = false
	closeErr := p.file.Close()
	if _, err := os.Lstat(p.path); errors.Is(err, fs.ErrNotExist) {
		return nil
	} else if err != nil {
		return ioError(ioErrorText(err))
	}
	if err := p.validate(); err != nil {
		return err
	}
	if err := os.Remove(p.path); err != nil {
		return ioError(ioErrorText(err))
	}
	if closeErr != nil && !errors.Is(closeErr, os.ErrClosed) {
		return ioError(ioErrorText(closeErr))
	}
	return nil
}

// withCapturePermit runs operation under one publication permit.
func withCapturePermit(control *captureExportControl, until time.Time, operation func() error) error {
	release, err := captureOperationPermit(control, until)
	if err != nil {
		return err
	}
	defer release()
	return operation()
}

func capturePublishFile(path string, data []byte, until time.Time, control *captureExportControl, hooks nativeCaptureExportHooks, boundary nativeCaptureExportBoundary) error {
	directory := filepath.Dir(path)
	directoryIdentity, err := capturePrivateDirectory(directory)
	if err != nil {
		return err
	}
	var temporary *publicationTemporary
	if err := withCapturePermit(control, until, func() error {
		var err error
		temporary, err = createPublicationTemporary(path)
		return err
	}); err != nil {
		return err
	}
	result := func() error {
		for chunk := range slices.Chunk(data, 65536) {
			if err := withCapturePermit(control, until, func() error {
				if _, err := temporary.file.Write(chunk); err != nil {
					return ioError(ioErrorText(err))
				}
				return nil
			}); err != nil {
				return err
			}
		}
		if err := withCapturePermit(control, until, func() error {
			if err := temporary.file.Sync(); err != nil {
				return ioError(ioErrorText(err))
			}
			return nil
		}); err != nil {
			return err
		}
		current := until
		if control != nil {
			current = control.deadline(until)
		}
		if err := hooks.before(boundary, current); err != nil {
			return err
		}
		if err := withCapturePermit(control, until, func() error { return nil }); err != nil {
			return err
		}
		identity, err := capturePrivateDirectory(directory)
		if err != nil {
			return err
		}
		if err := ensure(identity == directoryIdentity, "capture publication directory changed"); err != nil {
			return err
		}
		if err := temporary.validate(); err != nil {
			return err
		}
		if err := withCapturePermit(control, until, func() error {
			if err := os.Link(temporary.path, path); err != nil {
				return ioError(ioErrorText(err))
			}
			return nil
		}); err != nil {
			return err
		}
		if err := temporary.cleanup(); err != nil {
			return err
		}
		var handle *os.File
		if err := withCapturePermit(control, until, func() error {
			var err error
			if handle, err = os.Open(directory); err != nil {
				return ioError(ioErrorText(err))
			}
			return nil
		}); err != nil {
			return err
		}
		defer func() { _ = handle.Close() }()
		return withCapturePermit(control, until, func() error {
			if err := handle.Sync(); err != nil {
				return ioError(ioErrorText(err))
			}
			return nil
		})
	}()
	return mergeCleanup(result, temporary.cleanup())
}

func capturePublishPrivate(path string, data []byte, until time.Time) error {
	return capturePublishFile(path, data, until, nil, inertCaptureExportHooks{}, nativeCaptureExportArtifactPublish)
}

func publishNativeCapture(directory string, result *nativeCaptureResult, limits nativeCaptureLimits, until time.Time, control *captureExportControl, hooks nativeCaptureExportHooks) error {
	if err := runtimeCheckDeadline(time.Now(), until); err != nil {
		return err
	}
	if err := limits.validate(); err != nil {
		return err
	}
	directoryIdentity, err := capturePrivateDirectory(directory)
	if err != nil {
		return err
	}
	if err := validateCaptureID(result.CaptureID); err != nil {
		return err
	}
	complete := result.Status == nativeCaptureStatusComplete
	if err := ensure(!complete || len(result.Samples) == 2, "complete capture requires a pair"); err != nil {
		return err
	}
	state := "incomplete"
	if complete {
		state = "complete"
	}
	artifactPath := filepath.Join(directory, result.CaptureID+"."+state+".json")
	manifestPath := filepath.Join(directory, result.CaptureID+".manifest.json")
	for _, path := range []string{artifactPath, manifestPath} {
		if _, err := os.Lstat(path); err == nil {
			return appError("capture publication collision")
		} else if !errors.Is(err, fs.ErrNotExist) {
			return ioError(ioErrorText(err))
		}
	}
	release, err := capturePublicationPermit(control, hooks, nativeCaptureExportEncode, until)
	if err != nil {
		return err
	}
	encoded, err := encodeCaptureBounded(result, limits.maxEncodedResultBytes)
	release()
	if err != nil {
		return err
	}
	unchanged := func() error {
		identity, err := capturePrivateDirectory(directory)
		if err != nil {
			return err
		}
		return ensure(identity == directoryIdentity, "capture publication directory changed")
	}
	if err := unchanged(); err != nil {
		return err
	}
	if err := capturePublishFile(artifactPath, encoded, until, control, hooks, nativeCaptureExportArtifactPublish); err != nil {
		return err
	}
	manifest := jsonObject{
		{"schema_version", 1},
		{"capture_id", result.CaptureID},
		{"artifact_ref", filepath.Base(artifactPath)},
		{"artifact_sha256", sha256Hex(encoded)},
		{"artifact_bytes", len(encoded)},
		{"collection_status", result.Status},
		{"publication_status", state},
		{"producer_ref", result.producerRef},
		{"diagnostic", result.Diagnostic},
	}
	if err := unchanged(); err != nil {
		return err
	}
	if err := capturePublishFile(manifestPath, jsonValue(manifest), until, control, hooks, nativeCaptureExportManifestPublish); err != nil {
		return err
	}
	if control != nil {
		return control.acknowledge()
	}
	return nil
}

type capturePolicyFile struct {
	SchemaVersion uint32        `json:"schema_version"`
	Masks         []captureMask `json:"masks"`
}

// captureSystemClock is the system clock with a selected shutdown unit.
type captureSystemClock struct{ unit time.Duration }

func (captureSystemClock) now() time.Time     { return systemClock{}.now() }
func (captureSystemClock) wallNow() time.Time { return systemClock{}.wallNow() }
func (captureSystemClock) monotonicSample() (monotonicSample, error) {
	return systemClock{}.monotonicSample()
}
func (captureSystemClock) sleep(duration time.Duration)  { systemClock{}.sleep(duration) }
func (c captureSystemClock) shutdownUnit() time.Duration { return c.unit }

// captureVerificationSource is the run a capture configuration selects, with its settings.
type captureVerificationSource struct {
	config     nativeCaptureConfig
	settings   settings
	run        runRecord
	descriptor launchDescriptor
}

func openCaptureVerificationSource(path string, limits nativeCaptureLimits) (captureVerificationSource, error) {
	config, err := readNativeCaptureConfig(path, limits)
	if err != nil {
		return captureVerificationSource{}, err
	}
	database, err := databasePath(config.DatabaseURL)
	if err != nil {
		return captureVerificationSource{}, err
	}
	selected := settingsForDatabase(database)
	conn, err := connect(config.DatabaseURL)
	if err != nil {
		return captureVerificationSource{}, err
	}
	defer func() { _ = conn.close() }()
	descriptor, err := loadDescriptor(context.Background(), conn, config.DescriptorPath)
	if err != nil {
		return captureVerificationSource{}, err
	}
	run, err := getRun(context.Background(), conn, descriptor.Identity.RunID)
	if err != nil {
		return captureVerificationSource{}, err
	}
	return captureVerificationSource{config: config, settings: selected, run: run, descriptor: descriptor}, nil
}

func runNativeCaptureVerification(path string) (int32, error) {
	return runNativeCaptureVerificationWithExportHooks(path, inertCaptureExportHooks{}, time.Second)
}

func runNativeCaptureVerificationWithExportHooks(path string, hooks nativeCaptureExportHooks, shutdownUnit time.Duration) (int32, error) {
	limits := nativeCaptureLimitsDefault()
	source, err := openCaptureVerificationSource(path, limits)
	if err != nil {
		return 0, err
	}
	config := source.config
	if err := ensure(source.run.runID == config.ExpectedRunID && source.run.launchNonce == config.ExpectedLaunchNonce, "verification configuration run/nonce mismatch"); err != nil {
		return 0, err
	}
	catalog := profileCatalogBundled()
	data, err := captureReadPrivate(config.CapturePolicyPath, limits.maxConfigBytes)
	if err != nil {
		return 0, err
	}
	var policyFile capturePolicyFile
	if err := decodeStrict(data, &policyFile); err != nil {
		return 0, ioError(err.Error())
	}
	if err := ensure(policyFile.SchemaVersion == 1, "capture policy schema mismatch"); err != nil {
		return 0, err
	}
	policy, err := nativeCapturePolicyFromLaunch(policyFile.Masks, catalog)
	if err != nil {
		return 0, err
	}
	port, receiver, err := nativeCaptureChannel(source.run, policy, limits)
	if err != nil {
		return 0, err
	}
	producer, err := readNativeCaptureProducer(config.ProducerManifestPath, limits)
	if err != nil {
		return 0, err
	}
	port.serviceProducer = &producer
	exporter := installCaptureExporter(port, receiver, config, limits, hooks)
	status, serviceErr := runService(source.settings, config.DescriptorPath, runnerDependencies{
		catalog:           catalog,
		clock:             captureSystemClock{unit: shutdownUnit},
		diagnostics:       newRunnerDiagnostics(),
		capture:           port,
		terminalCallbacks: port.titles,
	})
	exported := exporter.finish(status, serviceErr)
	switch {
	case serviceErr == nil && exported == nil:
		return status, nil
	case serviceErr == nil && status != 0:
		return 0, appErrorf("capture service exited with status %d", status).withCleanup(cafleetErrorOf(exported).message())
	case serviceErr == nil:
		return 0, exported
	case exported != nil:
		return 0, cafleetErrorOf(serviceErr).withCleanup(cafleetErrorOf(exported).message())
	default:
		return 0, serviceErr
	}
}

// captureStagedWrite publishes data at path atomically, as privateFixtureWrite, for code that
// runs without a *testing.T: waiters poll these paths from other processes.
func captureStagedWrite(path string, data []byte) {
	staged := filepath.Join(filepath.Dir(path), fmt.Sprintf(".%s.partial-%d-%d", filepath.Base(path), os.Getpid(), stagedWrites.Add(1)))
	file, err := os.OpenFile(staged, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		panic(err)
	}
	if _, err := file.Write(data); err != nil {
		panic(err)
	}
	if err := file.Sync(); err != nil {
		panic(err)
	}
	if err := file.Close(); err != nil {
		panic(err)
	}
	if err := os.Link(staged, path); err != nil {
		panic(err)
	}
	if err := os.Remove(staged); err != nil {
		panic(err)
	}
}

// captureServiceFixture configures the synthetic native child with a private capture
// configuration, which selects a capture child for the runner.
func captureServiceFixture(t *testing.T, fixture *runnerFixture, spec *spawnSpec) {
	t.Helper()
	armPath := filepath.Join(fixture.dir, "capture-arm.json")
	if err := os.Chmod(filepath.Dir(armPath), 0o700); err != nil {
		t.Fatal(err)
	}
	if info, err := os.Lstat(filepath.Dir(armPath)); err != nil || info.Mode().Perm() != 0o700 {
		t.Fatalf("owned capture arm parent is private: %v %v", info, err)
	}
	syntheticObservationServices(t, fixture, spec)
	coverage := profileCatalogBundled().capabilities(codingAgentCodex).detection
	if _, err := sqlExecute(fixture.conn, "UPDATE member_runs SET coverage=?1 WHERE run_id=?2", coverage.asStr(), fixture.run.runID); err != nil {
		t.Fatal(err)
	}
	fixture.refresh(t)
	evidence := filepath.Join(fixture.dir, "synthetic-source.json")
	privateFixtureWrite(t, evidence, []byte(`{"kind":"synthetic","version":"0.0.1-synthetic"}`))
	digest := func(path string) string {
		value, err := sha256FileHex(path)
		if err != nil {
			t.Fatal(err)
		}
		return value
	}
	current, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	executable := spec.argv[0]
	producer := jsonObject{
		{"schema_version", 1},
		{"kind", "synthetic"},
		{"source_manifest_ref", evidence},
		{"source_manifest_sha256", digest(evidence)},
		{"libtest_executable_path", current},
		{"libtest_executable_sha256", libtestDigest()},
		{"parser_version", "0.18.1"},
		{"collector_version", "1"},
		{"catalog_origin", "bundled_unknown"},
		{"native_executable", jsonObject{{"path", executable}, {"sha256", digest(executable)}, {"version", "0.0.1-synthetic"}, {"version_evidence_ref", evidence}}},
		{"launch_evidence_ref", evidence},
		{"environment_evidence_ref", evidence},
	}
	producerPath := filepath.Join(fixture.dir, "capture-producer.json")
	privateFixtureWrite(t, producerPath, jsonValue(producer))
	policy := filepath.Join(fixture.dir, "capture-policy.json")
	privateFixtureWrite(t, policy, []byte(`{"schema_version":1,"masks":[]}`))
	output := filepath.Join(fixture.dir, "capture-output")
	if err := os.Mkdir(output, 0o700); err != nil {
		t.Fatal(err)
	}
	paths, err := runtimePathsForRun(fixture.run.databaseIdentityJSON, fixture.run.runID)
	if err != nil {
		t.Fatal(err)
	}
	config := nativeCaptureConfig{
		SchemaVersion:        1,
		DatabaseURL:          databaseURLFor(fixture.databaseFile(t)),
		DescriptorPath:       paths.descriptor,
		ExpectedRunID:        fixture.run.runID,
		ExpectedLaunchNonce:  fixture.run.launchNonce,
		ArmPath:              armPath,
		OutputDirectory:      output,
		ProducerManifestPath: producerPath,
		CapturePolicyPath:    policy,
	}
	privateFixtureWrite(t, filepath.Join(fixture.dir, "capture-config.json"), jsonValue(config))
}

// captureChildCommand is the child test a capture fixture's runner runs: a collector review
// entry, or the native capture service.
func captureChildCommand(t *testing.T, dir string) (*exec.Cmd, bool) {
	t.Helper()
	config := filepath.Join(dir, "capture-config.json")
	if _, err := os.Stat(config); err != nil {
		if _, entryErr := os.Stat(filepath.Join(dir, "collector-review-entry")); entryErr == nil {
			t.Fatal("collector review requires its private capture configuration")
		}
		return nil, false
	}
	child := TestRuntimeObservation_NativeCapture_Verification_NativeCaptureServiceChild
	if entry, err := os.ReadFile(filepath.Join(dir, "collector-review-entry")); err == nil {
		switch string(entry) {
		case "resize":
			child = TestRuntimeObservation_NativeCapture_Verification_Tests_RunnerCollectorResizeChild
		case "export":
			child = TestRuntimeObservation_NativeCapture_Exporter_Tests_RunnerCollectorExportChild
		default:
			t.Fatalf("unknown owned collector entry: %s", entry)
		}
	}
	command := childTestCommand(child)
	command.Env = append(command.Env, nativeCaptureEnv+"="+config)
	return command, true
}

// startCaptureRunning is startRunning's whole start result: the identity, runner and child.
func startCaptureRunning(f *serviceFixture) jsontext.Value {
	f.t.Helper()
	f.prepare()
	f.publish()
	reply, err := f.start()
	started := replyResult(f.t, reply, err)
	f.readUntil("review-native-ready")
	return started
}

func startedIdentity(t *testing.T, started jsontext.Value) controlIdentity {
	t.Helper()
	var identity controlIdentity
	if err := json.Unmarshal(jsonAt(t, started, "identity"), &identity); err != nil {
		t.Fatal(err)
	}
	return identity
}

func waitCapturePath(t *testing.T, path string, until time.Time) {
	t.Helper()
	for {
		if _, err := os.Stat(path); err == nil {
			return
		}
		if !time.Now().Before(until) {
			t.Fatalf("%s did not appear", path)
		}
		time.Sleep(5 * time.Millisecond)
	}
}

func readCaptureJSON(t *testing.T, path string) jsontext.Value {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	value := jsontext.Value(data)
	if !value.IsValid() {
		t.Fatalf("%s is not JSON: %q", path, data)
	}
	return value
}

func captureU64(t *testing.T, value jsontext.Value, path ...string) uint64 {
	t.Helper()
	number, ok := jsonUint64(jsonAt(t, value, path...))
	if !ok {
		t.Fatalf("%v is not an unsigned integer in %s", path, value)
	}
	return number
}

func TestRuntimeObservation_NativeCapture_Tests_ServiceCapture_ServicePrivateCapturePublishesManifestLastAndKeepsNativeWorkRunning(t *testing.T) {
	t.Parallel()
	f := configuredServiceFixture(t, false, captureServiceFixture)
	started := startCaptureRunning(f)
	identity := startedIdentity(t, started)
	armAt := time.Now().UTC()
	privateFixtureWrite(t, f.path("capture-arm.json"), jsonValue(nativeCaptureArmFor("service-pair", identity, uint64(realTimeBound/time.Millisecond))))
	output := f.path("capture-output")
	manifestPath := filepath.Join(output, "service-pair.manifest.json")
	waitCapturePath(t, manifestPath, time.Now().Add(2*realTimeBound))
	manifest := readCaptureJSON(t, manifestPath)
	if !jsonIsString(jsonIndex(manifest, "publication_status"), "complete") {
		t.Fatalf("manifest %s", manifest)
	}
	artifactPath := filepath.Join(output, "service-pair.complete.json")
	artifact, err := os.ReadFile(artifactPath)
	if err != nil {
		t.Fatalf("manifest publication follows the complete synced artifact: %v", err)
	}
	if captureU64(t, manifest, "artifact_bytes") != uint64(len(artifact)) || !jsonIsString(jsonIndex(manifest, "artifact_sha256"), sha256Hex(artifact)) {
		t.Fatalf("manifest %s describes another artifact", manifest)
	}
	result := jsontext.Value(artifact)
	samples, ok := jsonArray(jsonIndex(result, "samples"))
	if !jsonIsString(jsonIndex(result, "status"), "complete") || !ok || len(samples) != 2 {
		t.Fatalf("result %s", result)
	}
	source := jsonIndex(result, "source")
	if !jsonIsString(jsonAt(t, source, "producer", "kind"), "synthetic") || !jsonIsString(jsonAt(t, source, "producer", "catalog_origin"), "bundled_unknown") {
		t.Fatalf("producer %s", jsonIndex(source, "producer"))
	}
	assertJSONEqual(t, jsonIndex(source, "runner"), jsonAt(t, started, "runner"), "source runner")
	if !jsonIsString(jsonIndex(source, "launch_nonce"), identity.LaunchNonce) {
		t.Fatalf("source nonce %s", source)
	}
	clockRecord := jsonIndex(source, "clock")
	clockText := strings.ToLower(string(clockRecord))
	if clockRecord.Kind() != '{' {
		t.Fatalf("source clock %s", clockRecord)
	}
	for _, attribution := range []string{"real", "monotonic", "utc", "origin"} {
		if !strings.Contains(clockText, attribution) {
			t.Fatalf("a real service clock records %s: %s", attribution, clockText)
		}
	}
	if strings.Contains(clockText, "deterministic") {
		t.Fatalf("a real service clock is not deterministic: %s", clockText)
	}
	firstEnd := captureU64(t, samples[0], "monotonic_end_ns")
	secondStart := captureU64(t, samples[1], "monotonic_start_ns")
	if secondStart < firstEnd || secondStart-firstEnd < 100_000_000 {
		t.Fatalf("samples %d..%d are closer than the minimum interval", firstEnd, secondStart)
	}
	collectedAt := time.Now().UTC()
	for _, sample := range samples {
		if captureU64(t, sample, "monotonic_end_ns") < captureU64(t, sample, "monotonic_start_ns") {
			t.Fatalf("sample %s ends before it starts", sample)
		}
		text, _ := jsonString(jsonIndex(sample, "observed_at"))
		observed, err := time.Parse(time.RFC3339Nano, text)
		if err != nil || observed.Before(armAt.Truncate(time.Microsecond)) || observed.After(collectedAt) {
			t.Fatalf("sample observed at %q outside [%v, %v]: %v", text, armAt, collectedAt, err)
		}
	}
	assertJSONEqual(t, jsonAt(t, source, "child", "pid"), jsonAt(t, started, "child", "pid"), "source child pid")
	if owner, _ := jsonInt64(jsonIndex(source, "owner_epoch")); owner != identity.OwnerEpoch {
		t.Fatalf("source owner epoch %d, want %d", owner, identity.OwnerEpoch)
	}
	if status, exited, err := f.child.tryWait(); err != nil || exited {
		t.Fatalf("the runner keeps running after the capture: %v %v", status, err)
	}
	pairIdentity := func() [2]capturedOutput {
		return [2]capturedOutput{captureOutput(t, manifestPath), captureOutput(t, artifactPath)}
	}
	accepted := pairIdentity()
	f.health()
	f.assertManualIO()
	f.finish()
	for index, file := range accepted {
		file.assertUnchanged(t)
		_ = accepted[index].pin.Close()
	}
}

// capturedOutput is a published file's identity and bytes, pinned open.
type capturedOutput struct {
	path     string
	identity captureFileIdentity
	data     []byte
	pin      *os.File
}

func captureOutput(t *testing.T, path string) capturedOutput {
	t.Helper()
	file, err := os.OpenFile(path, os.O_RDONLY|syscall.O_NOFOLLOW, 0)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = file.Close() })
	info, err := file.Stat()
	if err != nil || !info.Mode().IsRegular() {
		t.Fatalf("owned publication snapshot %s is a regular file: %v", path, err)
	}
	data, err := io.ReadAll(file)
	if err != nil {
		t.Fatal(err)
	}
	return capturedOutput{path: path, identity: captureFileIdentityOf(info), data: data, pin: file}
}

func (o capturedOutput) assertUnchanged(t *testing.T) {
	t.Helper()
	info, err := os.Lstat(o.path)
	if err != nil || !info.Mode().IsRegular() {
		t.Fatalf("preserved %s is a regular file: %v", o.path, err)
	}
	if captureFileIdentityOf(info) != o.identity {
		t.Fatalf("preserved identity: %s", o.path)
	}
	data, err := os.ReadFile(o.path)
	if err != nil || !bytes.Equal(data, o.data) {
		t.Fatalf("preserved bytes: %s: %v", o.path, err)
	}
}

type exportServiceReturn struct {
	at       time.Time
	deadline opt[time.Time]
	status   opt[int32]
	err      opt[string]
}

// exportTestHooks records the export boundaries it enters, stalls at one until released, and
// marks its final release as Rust's Drop does.
type exportTestHooks struct {
	directory  string
	stall      opt[nativeCaptureExportBoundary]
	mu         sync.Mutex
	returned   *exportServiceReturn
	references atomic.Int32
}

func (h *exportTestHooks) retain() { h.references.Add(1) }

func (h *exportTestHooks) release() {
	if h.references.Add(-1) != 0 {
		return
	}
	file, err := os.OpenFile(filepath.Join(h.directory, "export-hooks-dropped"), os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		fmt.Fprintf(os.Stderr, "owned exporter completion marker failed: %v\n", err)
		return
	}
	_ = file.Close()
}

func (h *exportTestHooks) serviceReturned(storageDeadline opt[time.Time], status int32, err error) {
	observed := &exportServiceReturn{at: time.Now(), deadline: storageDeadline}
	if err == nil {
		observed.status = some(status)
	} else {
		observed.err = some(cafleetErrorOf(err).message())
	}
	h.mu.Lock()
	defer h.mu.Unlock()
	if h.returned != nil {
		panic("one actual service-return notification")
	}
	h.returned = observed
}

func (h *exportTestHooks) before(boundary nativeCaptureExportBoundary, deadline time.Time) error {
	remaining := max(time.Until(deadline), 0)
	captureStagedWrite(filepath.Join(h.directory, "export-entered-"+boundary.name()),
		jsonValue(jsonObject{{"boundary", boundary.name()}, {"remaining_ms", remaining.Milliseconds()}}))
	if stall, set := h.stall.get(); set && stall == boundary {
		emergency := time.Now().Add(10 * time.Second)
		for {
			if _, err := os.Stat(filepath.Join(h.directory, "export-release")); err == nil {
				break
			}
			if err := ensure(time.Now().Before(emergency), "owned export stall exceeded its emergency bound"); err != nil {
				return err
			}
			time.Sleep(5 * time.Millisecond)
		}
	}
	return nil
}

func TestRuntimeObservation_NativeCapture_Exporter_Tests_RunnerCollectorExportChild(t *testing.T) {
	t.Parallel()
	skipUnlessChildTest(t)
	path := os.Getenv(nativeCaptureEnv)
	if path == "" {
		panic("owned exporter configuration")
	}
	directory := filepath.Dir(path)
	plan := readCaptureJSON(t, filepath.Join(directory, "export-test-plan.json"))
	hooks := &exportTestHooks{directory: directory}
	hooks.references.Store(1)
	switch stall := jsonIndex(plan, "stall"); {
	case jsonIsString(stall, "encode"):
		hooks.stall = some(nativeCaptureExportEncode)
	case jsonIsString(stall, "manifest"):
		hooks.stall = some(nativeCaptureExportManifestPublish)
	case jsonIsNull(stall):
	default:
		panic(fmt.Sprintf("owned export stall selection required: %s", stall))
	}
	unitMS, ok := jsonUint64(jsonIndex(plan, "shutdown_unit_ms"))
	if !ok {
		panic("the export plan names its shutdown unit")
	}
	begin := time.Now()
	status, err := runNativeCaptureVerificationWithExportHooks(path, hooks, time.Duration(unitMS)*time.Millisecond) //nolint:gosec // a small plan value
	completed := time.Now()
	since := func(at time.Time) int64 { return at.Sub(begin).Nanoseconds() }
	service := jsonValue(nil)
	hooks.mu.Lock()
	if observed := hooks.returned; observed != nil {
		storageDeadline, exhausted := any(nil), any(nil)
		if deadline, set := observed.deadline.get(); set {
			storageDeadline, exhausted = since(deadline), !observed.at.Before(deadline)
		}
		service = jsonValue(jsonObject{
			{"returned_ns", since(observed.at)},
			{"storage_deadline_ns", storageDeadline},
			{"deadline_exhausted", exhausted},
			{"status", observed.status},
			{"error", observed.err},
			{"completion_after_service_ns", completed.Sub(observed.at).Nanoseconds()},
		})
	}
	hooks.mu.Unlock()
	record := jsonObject{{"result", "success"}, {"status", status}, {"elapsed_ms", time.Since(begin).Milliseconds()}}
	if err != nil {
		record = jsonObject{{"result", "error"}, {"message", cafleetErrorOf(err).message()}, {"elapsed_ms", time.Since(begin).Milliseconds()}}
	}
	record = append(record, jsonMember{"completed_ns", since(completed)}, jsonMember{"service", service})
	returned := filepath.Join(directory, "export-helper-returned.json")
	pending := strings.TrimSuffix(returned, filepath.Ext(returned)) + ".pending"
	captureStagedWrite(pending, jsonValue(record))
	if err := os.Rename(pending, returned); err != nil {
		panic(err)
	}
	hooks.release()
	until := time.Now().Add(10 * time.Second)
	for {
		if _, statErr := os.Stat(filepath.Join(directory, "export-hooks-dropped")); statErr == nil {
			break
		}
		if !time.Now().Before(until) {
			panic("owned exporter did not release its final hook reference")
		}
		time.Sleep(5 * time.Millisecond)
	}
	if refusal, _ := jsonBool(jsonIndex(plan, "startup_refusal")); refusal {
		if err == nil {
			panic("an invalid producer fails before native startup")
		}
		os.Exit(1)
	}
	os.Exit(0)
}

// releaseExport releases the stalled export hooks, at the latest at test cleanup.
type releaseExport struct {
	path     string
	released bool
}

func newReleaseExport(t *testing.T, f *serviceFixture) *releaseExport {
	t.Helper()
	release := &releaseExport{path: f.path("export-release")}
	t.Cleanup(func() {
		if release.released {
			return
		}
		file, err := os.OpenFile(release.path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
		if err != nil {
			t.Logf("owned export emergency release failed: %v", err)
			return
		}
		_ = file.Close()
	})
	return release
}

func (r *releaseExport) release(t *testing.T) {
	t.Helper()
	privateFixtureWrite(t, r.path, []byte("owned exporter release after helper return"))
	r.released = true
}

// storageBlocker holds the run database's write lock until released.
func storageBlocker(t *testing.T, f *serviceFixture) func() {
	t.Helper()
	conn, err := connect(databaseURLFor(f.fixture.databaseFile(t)))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := sqlExecute(conn, "BEGIN IMMEDIATE"); err != nil {
		t.Fatal(err)
	}
	released := false
	release := func() {
		if released {
			return
		}
		released = true
		if _, err := sqlExecute(conn, "ROLLBACK"); err != nil {
			t.Logf("owned storage lock release failed: %v", err)
		}
		_ = conn.close()
	}
	t.Cleanup(release)
	return release
}

func configureCaptureExport(t *testing.T, fixture *runnerFixture, spec *spawnSpec, stall opt[string], startupRefusal bool) {
	t.Helper()
	captureServiceFixture(t, fixture, spec)
	privateFixtureWrite(t, filepath.Join(fixture.dir, "collector-review-entry"), []byte("export"))
	unit := 1000
	if stall.set {
		unit = 100
	}
	privateFixtureWrite(t, filepath.Join(fixture.dir, "export-test-plan.json"),
		jsonValue(jsonObject{{"stall", stall}, {"startup_refusal", startupRefusal}, {"shutdown_unit_ms", unit}}))
}

// stalledCaptureExport is a running exporter service that stalls at the stall export boundary.
func stalledCaptureExport(t *testing.T, stall string) *serviceFixture {
	t.Helper()
	return configuredServiceFixture(t, false, func(t *testing.T, fixture *runnerFixture, spec *spawnSpec) {
		configureCaptureExport(t, fixture, spec, some(stall), false)
	})
}

func armCaptureExport(t *testing.T, f *serviceFixture, started jsontext.Value) {
	t.Helper()
	privateFixtureWrite(t, f.path("capture-arm.json"), jsonValue(nativeCaptureArmFor("export-pair", startedIdentity(t, started), 30000)))
}

func exportHelperRecord(t *testing.T, f *serviceFixture) jsontext.Value {
	t.Helper()
	return readCaptureJSON(t, f.path("export-helper-returned.json"))
}

func assertExportUnfinished(t *testing.T, record jsontext.Value) {
	t.Helper()
	message, ok := jsonString(jsonIndex(record, "message"))
	message = strings.ToLower(message)
	if !jsonIsString(jsonIndex(record, "result"), "error") || !ok || !strings.Contains(message, "unfinished") ||
		(!strings.Contains(message, "export") && !strings.Contains(message, "capture")) {
		t.Fatalf("an explicit unfinished exporter error: %s", record)
	}
}

func assertExportOriginalCompletionBound(t *testing.T, record jsontext.Value) {
	t.Helper()
	deadline, ok := jsonUint64(jsonAt(t, record, "service", "storage_deadline_ns"))
	if !ok {
		t.Fatalf("the original service storage deadline: %s", record)
	}
	if completed := captureU64(t, record, "completed_ns"); completed > deadline+100_000_000 {
		t.Fatalf("helper completion stays within the original deadline plus a 100 ms scheduling allowance: %s", record)
	}
}

func assertExportExpiredServiceFailure(t *testing.T, record jsontext.Value) {
	t.Helper()
	assertExportOriginalCompletionBound(t, record)
	service := jsonIndex(record, "service")
	if exhausted, _ := jsonBool(jsonIndex(service, "deadline_exhausted")); !exhausted {
		t.Fatalf("the actual service return exhausts its own storage deadline: %s", record)
	}
	if status, _ := jsonInt64(jsonIndex(service, "status")); status != 1 {
		t.Fatalf("a native zero with blocked storage yields the incomplete-service status: %s", record)
	}
	if !jsonIsNull(jsonIndex(service, "error")) {
		t.Fatalf("the uncombined actual service result is recorded: %s", record)
	}
	if captureU64(t, service, "returned_ns") < captureU64(t, service, "storage_deadline_ns") {
		t.Fatalf("the service returns at its storage deadline: %s", record)
	}
	if captureU64(t, service, "completion_after_service_ns") > 100_000_000 {
		t.Fatalf("an expired deadline permits only the 100 ms scheduling allowance after the service return: %s", record)
	}
	assertExportUnfinished(t, record)
	message, _ := jsonString(jsonIndex(record, "message"))
	retained := slices.ContainsFunc(strings.Split(strings.ToLower(message), "\n"), func(line string) bool {
		return strings.Contains(line, "service") && slices.Contains(strings.FieldsFunc(line, func(r rune) bool { return r < '0' || r > '9' }), "1")
	})
	if !retained {
		t.Fatalf("the combined helper result keeps the observed service status beside the unfinished export: %s", record)
	}
}

func captureFinalManifest(f *serviceFixture) string {
	return f.path("capture-output/export-pair.manifest.json")
}

func captureArtifactPath(f *serviceFixture) string {
	return f.path("capture-output/export-pair.complete.json")
}

func seedUnrelatedOutput(t *testing.T, f *serviceFixture) capturedOutput {
	t.Helper()
	path := f.path("capture-output/unrelated-output.txt")
	privateFixtureWrite(t, path, []byte("pre-existing unrelated output identity"))
	return captureOutput(t, path)
}

func captureOutputPaths(t *testing.T, f *serviceFixture) []string {
	t.Helper()
	entries, err := os.ReadDir(f.path("capture-output"))
	if err != nil {
		t.Fatal(err)
	}
	paths := make([]string, 0, len(entries))
	for _, entry := range entries {
		paths = append(paths, f.path(filepath.Join("capture-output", entry.Name())))
	}
	return paths
}

func temporaryCaptureOutputs(t *testing.T, f *serviceFixture, preserved ...capturedOutput) []capturedOutput {
	t.Helper()
	var temporaries []capturedOutput
	for _, path := range captureOutputPaths(t, f) {
		if !slices.ContainsFunc(preserved, func(file capturedOutput) bool { return file.path == path }) {
			temporaries = append(temporaries, captureOutput(t, path))
		}
	}
	return temporaries
}

func assertCaptureTemporariesRemoved(t *testing.T, temporaries []capturedOutput) {
	t.Helper()
	for _, file := range temporaries {
		if _, err := os.Lstat(file.path); !errors.Is(err, fs.ErrNotExist) {
			t.Fatalf("the worker removes its still-matching temporary before fixture cleanup: %s: %v", file.path, err)
		}
	}
}

func assertOnlyPreservedOutputs(t *testing.T, f *serviceFixture, preserved ...capturedOutput) {
	t.Helper()
	kept := make([]string, 0, len(preserved))
	for _, file := range preserved {
		file.assertUnchanged(t)
		kept = append(kept, file.path)
	}
	slices.Sort(kept)
	if outputs := captureOutputPaths(t, f); !slices.Equal(outputs, kept) {
		t.Fatalf("the worker leaves no task-created temporary or late output: %v, want %v", outputs, kept)
	}
}

func assertPathAbsent(t *testing.T, path string) {
	t.Helper()
	if _, err := os.Lstat(path); !errors.Is(err, fs.ErrNotExist) {
		t.Fatalf("%s exists: %v", path, err)
	}
}

func assertStartedChildMatches(t *testing.T, started jsontext.Value, want bool) {
	t.Helper()
	matches, err := startedChild(t, jsonAt(t, started, "child")).matches()
	if err != nil || matches != want {
		t.Fatalf("the started native child matches = %v, want %v: %v", matches, want, err)
	}
}

func TestRuntimeObservation_NativeCapture_Exporter_Tests_RunnerCollectorEncodeAndPrepermitManifestStallsReturnUnfinishedAndRevokeLatePublication(t *testing.T) {
	t.Parallel()
	f := stalledCaptureExport(t, "encode")
	release := newReleaseExport(t, f)
	unrelated := seedUnrelatedOutput(t, f)
	started := startCaptureRunning(f)
	armCaptureExport(t, f, started)
	waitCapturePath(t, f.path("export-entered-Encode"), time.Now().Add(realTimeBound))
	assertPathAbsent(t, captureFinalManifest(f))
	temporaries := temporaryCaptureOutputs(t, f, unrelated)
	assertStartedChildMatches(t, started, true)
	f.assertManualIO()
	shutdown := time.Now()
	f.writeOuter("finish\n")
	waitCapturePath(t, f.path("export-helper-returned.json"), shutdown.Add(realTimeBound))
	assertExportUnfinished(t, exportHelperRecord(t, f))
	assertExportOriginalCompletionBound(t, exportHelperRecord(t, f))
	assertStartedChildMatches(t, started, false)
	assertPathAbsent(t, captureFinalManifest(f))
	release.release(t)
	waitCapturePath(t, f.path("export-hooks-dropped"), time.Now().Add(realTimeBound))
	f.assertExitCode(0)
	assertCaptureTemporariesRemoved(t, temporaries)
	assertOnlyPreservedOutputs(t, f, unrelated)
	assertPathAbsent(t, captureFinalManifest(f))
	assertPathAbsent(t, f.path("export-entered-ArtifactPublish"))
}

func TestRuntimeObservation_NativeCapture_Exporter_Tests_RunnerCollectorCancelledManifestWorkerPreservesForeignIdentitiesAndCleansItsMatchingTemporaries(t *testing.T) {
	t.Parallel()
	f := stalledCaptureExport(t, "manifest")
	release := newReleaseExport(t, f)
	unrelated := seedUnrelatedOutput(t, f)
	started := startCaptureRunning(f)
	armCaptureExport(t, f, started)
	waitCapturePath(t, f.path("export-entered-ManifestPublish"), time.Now().Add(realTimeBound))
	assertPathAbsent(t, captureFinalManifest(f))
	artifact := captureOutput(t, captureArtifactPath(f))
	temporaries := temporaryCaptureOutputs(t, f, unrelated, artifact)
	shutdown := time.Now()
	f.writeOuter("finish\r")
	waitCapturePath(t, f.path("export-helper-returned.json"), shutdown.Add(realTimeBound))
	original := exportHelperRecord(t, f)
	assertExportUnfinished(t, original)
	assertExportOriginalCompletionBound(t, original)
	assertStartedChildMatches(t, started, false)

	replaceable := slices.IndexFunc(temporaries, func(file capturedOutput) bool {
		info, err := os.Lstat(file.path)
		if errors.Is(err, fs.ErrNotExist) {
			return false
		}
		if err != nil {
			t.Fatalf("owned temporary inspection failed: %v", err)
		}
		return captureFileIdentityOf(info) == file.identity
	})
	var foreignTemporary capturedOutput
	var displaced *[2]capturedOutput
	if replaceable >= 0 {
		replaced := temporaries[replaceable]
		temporaries = slices.Delete(temporaries, replaceable, replaceable+1)
		displacedPath := f.path("displaced-exporter-temporary")
		if err := os.Rename(replaced.path, displacedPath); err != nil {
			t.Fatal(err)
		}
		moved := captureOutput(t, displacedPath)
		if moved.identity != replaced.identity {
			t.Fatal("the displaced temporary keeps its identity")
		}
		privateFixtureWrite(t, replaced.path, []byte("foreign replacement at an exporter temporary name"))
		foreignTemporary = captureOutput(t, replaced.path)
		if foreignTemporary.identity == replaced.identity {
			t.Fatal("the foreign replacement is a new file")
		}
		displaced = &[2]capturedOutput{replaced, moved}
	} else {
		path := strings.TrimSuffix(captureFinalManifest(f), ".json") + ".provisional"
		assertPathAbsent(t, path)
		privateFixtureWrite(t, path, []byte("foreign file at the publisher temporary name"))
		foreignTemporary = captureOutput(t, path)
	}
	privateFixtureWrite(t, captureFinalManifest(f), []byte("pre-existing collision output before revoked worker resumes"))
	collision := captureOutput(t, captureFinalManifest(f))
	unrelatedTemporaryPath := f.path("capture-output/foreign.provisional")
	privateFixtureWrite(t, unrelatedTemporaryPath, []byte("unrelated provisional identity"))
	unrelatedTemporary := captureOutput(t, unrelatedTemporaryPath)

	release.release(t)
	waitCapturePath(t, f.path("export-hooks-dropped"), time.Now().Add(realTimeBound))
	f.assertExitCode(0)
	assertCaptureTemporariesRemoved(t, temporaries)
	assertOnlyPreservedOutputs(t, f, unrelated, artifact, foreignTemporary, collision, unrelatedTemporary)
	assertJSONEqual(t, exportHelperRecord(t, f), original, "a later worker completion does not acknowledge the unfinished invocation")
	if displaced != nil {
		replaced, moved := displaced[0], displaced[1]
		moved.assertUnchanged(t)
		if !bytes.Equal(moved.data, replaced.data) {
			t.Fatal("the displaced temporary keeps its bytes")
		}
		if err := os.Remove(moved.path); err != nil {
			t.Fatal(err)
		}
	}
	run := mustRun(t, f.fixture.conn, startedIdentity(t, started).RunID)
	if run.status != runStatusExited || run.exitCode != some[int64](0) {
		t.Fatalf("run %s exit %v", run.status.asStr(), run.exitCode)
	}
}

func TestRuntimeObservation_NativeCapture_Exporter_Tests_RunnerCollectorStartupFailureHasNoExportWaitOrPublication(t *testing.T) {
	t.Parallel()
	f := launchServiceFixture(t, false, func(t *testing.T, fixture *runnerFixture, spec *spawnSpec) {
		configureCaptureExport(t, fixture, spec, none[string](), true)
		path := filepath.Join(fixture.dir, "capture-producer.json")
		producer := jsonWith(readCaptureJSON(t, path), "collector_version", "invalid-owned-version")
		if err := os.WriteFile(path, producer, 0o600); err != nil {
			t.Fatal(err)
		}
	}, true, none[time.Time]())
	record := exportHelperRecord(t, f)
	if !jsonIsString(jsonIndex(record, "result"), "error") {
		t.Fatalf("record %s", record)
	}
	if !jsonIsNull(jsonIndex(record, "service")) {
		t.Fatalf("a pre-worker refusal invokes no actual service-return callback: %s", record)
	}
	if captureU64(t, record, "elapsed_ms") >= 1000 {
		t.Fatalf("an absent service deadline grants no new exporter budget: %s", record)
	}
	if message, _ := jsonString(jsonIndex(record, "message")); strings.Contains(message, "unfinished") {
		t.Fatalf("record %s", record)
	}
	for _, boundary := range []string{"Encode", "ArtifactPublish", "ManifestPublish"} {
		assertPathAbsent(t, f.path("export-entered-"+boundary))
	}
	assertPathAbsent(t, captureFinalManifest(f))
	if run := mustRun(t, f.fixture.conn, f.fixture.run.runID); run.childPID.set {
		t.Fatalf("a refused startup spawns no native child: %v", run.childPID)
	}
}

func TestRuntimeObservation_NativeCapture_Exporter_Tests_RunnerCollectorExhaustedServiceStorageBudgetAddsNoFreshExporterWait(t *testing.T) {
	t.Parallel()
	f := stalledCaptureExport(t, "encode")
	release := newReleaseExport(t, f)
	unrelated := seedUnrelatedOutput(t, f)
	started := startCaptureRunning(f)
	armCaptureExport(t, f, started)
	waitCapturePath(t, f.path("export-entered-Encode"), time.Now().Add(realTimeBound))
	temporaries := temporaryCaptureOutputs(t, f, unrelated)
	unblock := storageBlocker(t, f)
	shutdown := time.Now()
	f.writeOuter("finish\r")
	waitCapturePath(t, f.path("export-helper-returned.json"), shutdown.Add(realTimeBound))
	assertExportExpiredServiceFailure(t, exportHelperRecord(t, f))
	assertStartedChildMatches(t, started, false)
	assertPathAbsent(t, captureFinalManifest(f))
	unblock()
	release.release(t)
	waitCapturePath(t, f.path("export-hooks-dropped"), time.Now().Add(realTimeBound))
	f.assertExitCode(0)
	assertCaptureTemporariesRemoved(t, temporaries)
	assertOnlyPreservedOutputs(t, f, unrelated)
	assertPathAbsent(t, f.path("export-entered-ArtifactPublish"))
	assertPathAbsent(t, captureFinalManifest(f))
}

func TestRuntimeObservation_NativeCapture_Verification_Tests_RunnerCollectorCommonServiceArmsOnActualResizeAndPreservesTriggerGeometryAndSequence(t *testing.T) {
	t.Parallel()
	f := configuredServiceFixture(t, false, func(t *testing.T, fixture *runnerFixture, spec *spawnSpec) {
		captureServiceFixture(t, fixture, spec)
		privateFixtureWrite(t, filepath.Join(fixture.dir, "collector-review-entry"), []byte("resize"))
	})
	started := startCaptureRunning(f)
	before := captureU64(t, f.health(), "output_seq")
	process, _ := f.process.get()
	if matches, err := process.matches(); err != nil || !matches {
		t.Fatalf("the runner process matches: %v", err)
	}
	f.resize(25, 90)
	pairPath := f.path("capture-output/actual-resize-pair.json")
	waitCapturePath(t, pairPath, time.Now().Add(realTimeBound))
	pair := readCaptureJSON(t, pairPath)
	samples, ok := jsonArray(jsonIndex(pair, "samples"))
	if !jsonIsString(jsonIndex(pair, "status"), "complete") || !ok || len(samples) != 2 {
		t.Fatalf("pair %s", pair)
	}
	if !jsonIsString(jsonIndex(samples[0], "trigger"), "resize") || !jsonIsString(jsonIndex(samples[1], "trigger"), "timer") {
		t.Fatalf("triggers %s", pair)
	}
	for _, sample := range samples {
		if captureU64(t, sample, "rows") != 25 || captureU64(t, sample, "cols") != 90 || captureU64(t, sample, "screen_seq") != before+1 {
			t.Fatalf("sample %s, want 25x90 at screen %d", sample, before+1)
		}
	}
	firstEnd, secondStart := captureU64(t, samples[0], "monotonic_end_ns"), captureU64(t, samples[1], "monotonic_start_ns")
	if secondStart < firstEnd || secondStart-firstEnd < 100_000_000 {
		t.Fatalf("samples %d..%d are closer than the minimum interval", firstEnd, secondStart)
	}
	if !jsonIsString(jsonAt(t, pair, "source", "clock", "kind"), "real") {
		t.Fatalf("clock %s", jsonAt(t, pair, "source", "clock"))
	}
	assertJSONEqual(t, f.healthChild(), jsonAt(t, started, "child"), "health child")
	if status, exited, err := f.child.tryWait(); err != nil || exited {
		t.Fatalf("the runner keeps running: %v %v", status, err)
	}
	f.assertManualIO()
	f.finish()
}

// resizeArm requests a capture at the first resize detector step.
type resizeArm struct {
	receiver *nativeCaptureReceiver
	used     atomic.Bool
}

func (a *resizeArm) before(site observationPhase, operation observationOperation) error {
	if site != observationPhaseResize || operation != observationOperationDetector || a.used.Swap(true) {
		return nil
	}
	identity := a.receiver.finalizedIdentity()
	if identity == nil {
		panic("an actual running resize source")
	}
	return a.receiver.request(nativeCaptureRequest{
		captureID:       "actual-resize",
		runID:           identity.RunID,
		ownerEpoch:      identity.OwnerEpoch,
		minimumInterval: 100 * time.Millisecond,
		deadline:        time.Now().Add(2 * time.Second),
	})
}

func (*resizeArm) orderShutdownParsingAfterNativeExit(time.Time) bool { return false }

func TestRuntimeObservation_NativeCapture_Verification_Tests_RunnerCollectorResizeChild(t *testing.T) {
	t.Parallel()
	skipUnlessChildTest(t)
	limits := nativeCaptureLimitsDefault()
	source, err := openCaptureVerificationSource(os.Getenv(nativeCaptureEnv), limits)
	if err != nil {
		panic(err)
	}
	if source.run.runID != source.config.ExpectedRunID || source.run.launchNonce != source.config.ExpectedLaunchNonce {
		panic("the resize configuration selects its run")
	}
	catalog := profileCatalogBundled()
	policy, err := nativeCapturePolicyFromLaunch(nil, catalog)
	if err != nil {
		panic(err)
	}
	port, receiver, err := nativeCaptureChannel(source.run, policy, limits)
	if err != nil {
		panic(err)
	}
	producer, err := readNativeCaptureProducer(source.config.ProducerManifestPath, limits)
	if err != nil {
		panic(err)
	}
	if !jsonIsString(jsonIndex(producer.value, "kind"), "synthetic") {
		panic("the resize fixture's producer is synthetic")
	}
	port.serviceProducer = &producer
	observer := &resizeArm{receiver: receiver}
	var stop atomic.Bool
	resultPath := filepath.Join(source.config.OutputDirectory, "actual-resize-pair.json")
	collected := make(chan error, 1)
	go func() {
		until := time.Now().Add(8 * time.Second)
		for !stop.Load() && time.Now().Before(until) {
			leased, err := receiver.tryTake()
			if err != nil {
				collected <- err
				return
			}
			if result, set := leased.get(); set {
				collected <- capturePublishPrivate(resultPath, jsonValue(result), until)
				return
			}
			time.Sleep(2 * time.Millisecond)
		}
		collected <- appError("owned resize pair was not completed")
	}()
	status, err := runService(source.settings, source.config.DescriptorPath, runnerDependencies{
		catalog:           catalog,
		clock:             systemClock{},
		diagnostics:       newRunnerDiagnostics(),
		observationHooks:  observer,
		capture:           port,
		terminalCallbacks: port.titles,
	})
	stop.Store(true)
	if collectErr := <-collected; collectErr != nil {
		panic(collectErr)
	}
	if !observer.used.Load() || err != nil || status != 0 {
		panic(fmt.Sprintf("the resize service runs to a zero exit with its arm used: used=%v status=%d err=%v", observer.used.Load(), status, err))
	}
	os.Exit(0)
}

func TestRuntimeObservation_NativeCapture_Verification_NativeCaptureServiceChild(t *testing.T) {
	t.Parallel()
	skipUnlessChildTest(t)
	path := os.Getenv(nativeCaptureEnv)
	if path == "" {
		panic(nativeCaptureEnv + " is required for explicit native verification")
	}
	if !filepath.IsAbs(path) {
		panic("the capture configuration path is absolute")
	}
	config, err := readNativeCaptureConfig(path, nativeCaptureLimitsDefault())
	if err != nil {
		panic(err)
	}
	boundedDigest := func(path string) string {
		file, err := os.OpenFile(path, os.O_RDONLY|syscall.O_NOFOLLOW, 0)
		if err != nil {
			panic(err)
		}
		defer func() { _ = file.Close() }()
		data, err := io.ReadAll(io.LimitReader(file, 65537))
		if err != nil {
			panic(err)
		}
		if len(data) > 65536 {
			panic("bounded private outcome evidence required")
		}
		return sha256Hex(data)
	}
	configDigest := boundedDigest(path)
	producerDigest := boundedDigest(config.ProducerManifestPath)
	process, err := processIdentityCurrent()
	if err != nil {
		panic(err)
	}
	status, runErr := runNativeCaptureVerification(path)
	outcome := jsonObject{
		{"schema_version", 1},
		{"config_sha256", configDigest},
		{"run_id", config.ExpectedRunID},
		{"launch_nonce", config.ExpectedLaunchNonce},
		{"producer_sha256", producerDigest},
		{"process", process},
	}
	if runErr == nil {
		outcome = append(outcome, jsonMember{"result", "ok"}, jsonMember{"status", status}, jsonMember{"error", nil})
	} else {
		message := []rune(cafleetErrorOf(runErr).message())
		outcome = append(outcome, jsonMember{"result", "error"}, jsonMember{"status", nil}, jsonMember{"error", string(message[:min(len(message), 2048)])})
	}
	encoded := jsonValue(outcome)
	if len(encoded) > 16*1024 {
		panic("the service outcome fits its bound")
	}
	outcomePath := strings.TrimSuffix(path, filepath.Ext(path)) + ".service-outcome.json"
	if err := capturePublishPrivate(outcomePath, encoded, time.Now().Add(10*time.Second)); err != nil {
		panic(err)
	}
	if runErr != nil {
		panic("native capture service returned an error: " + cafleetErrorOf(runErr).message())
	}
	os.Exit(int(status))
}

// Committed native fixtures derive from these captures, so their visible paths must not name a home directory.
const driverNeutralRoot = "/tmp/cafleet-194"

// driverPrivateRoot holds case databases and sockets on the local filesystem: the repository
// mount reports root-created files as owned by another user, which the private ownership checks
// refuse. macOS reaches /tmp through the /private/tmp symlink, and the runner launch passes the
// canonical database path, so the private root is named canonically there.
func driverPrivateRoot() string {
	if runtime.GOOS == "darwin" {
		return "/private/tmp/cafleet-194-p"
	}
	return "/tmp/cafleet-194-p"
}

const (
	driverAnswerWindowMS    uint64 = 60_000
	driverToolFreePrompt           = "Reply with the single word ready and then wait."
	driverReadProbeContent         = "cafleet-199-read-probe"
	driverEditTargetContent        = "before"
	driverClaudeSeed               = "claude-seed"
	// Long enough that the displayed `Read(<path>)` operation exceeds one 120-column row.
	driverLongReadProbe    = "a-deeply-nested-directory-for-the-long-read-operation/second-level-directory-that-wraps-the-displayed-path/probe.txt"
	driverPanelProbe       = "panel-probe.txt"
	driverBatchedReadProbe = "batched-probe.txt"
	driverPanelCommitted   = "panel-committed.txt"
	driverConfigCheck      = "claude-config-check.json"
)

// One space and 400 `x` characters, so the command outgrows the form's command box at 57 columns.
var driverLongCommandArgument = "allow " + strings.Repeat("x", 400)

type driverMode uint8

const (
	driverModeNoAnswer driverMode = iota + 1
	driverModeAnswerStage
)

type driverFileProbeKind uint8

const (
	driverProbeRead driverFileProbeKind = iota + 1
	driverProbeEdit
	driverProbeWrite
	driverProbeGlob
	driverProbeGrep
	driverProbeBashTouch
)

// driverFileProbe is a file-tool probe; readProbe is the target of a driverProbeRead.
type driverFileProbe struct {
	kind      driverFileProbeKind
	readProbe string
}

var driverReadFileProbe = driverFileProbe{kind: driverProbeRead, readProbe: "probe.txt"}

func (p driverFileProbe) tool() string {
	switch p.kind {
	case driverProbeRead:
		return "Read"
	case driverProbeEdit:
		return "Edit"
	case driverProbeWrite:
		return "Write"
	case driverProbeGlob:
		return "Glob"
	case driverProbeGrep:
		return "Grep"
	case driverProbeBashTouch:
		return "Bash"
	default:
		panic("invalid driverFileProbeKind")
	}
}

func (p driverFileProbe) operation() string {
	switch p.kind {
	case driverProbeRead:
		return "read the file"
	case driverProbeEdit:
		return "replace the text before with after in the file"
	case driverProbeWrite:
		return "write the content cafleet-199-write-probe to the file"
	case driverProbeGlob:
		return "match the pattern *.txt in the directory"
	case driverProbeGrep:
		return "search for the pattern cafleet-199-read-probe in the directory"
	case driverProbeBashTouch:
		return "run exactly touch"
	default:
		panic("invalid driverFileProbeKind")
	}
}

func (p driverFileProbe) target() string {
	switch p.kind {
	case driverProbeRead:
		return p.readProbe
	case driverProbeEdit:
		return "edit-target.txt"
	case driverProbeWrite:
		return "write-target.txt"
	case driverProbeGlob, driverProbeGrep:
		return ""
	case driverProbeBashTouch:
		return "no-tip-access.txt"
	default:
		panic("invalid driverFileProbeKind")
	}
}

func (p driverFileProbe) probe() string {
	if p.kind == driverProbeRead {
		return p.readProbe
	}
	return "probe.txt"
}

// driverClaudeConfig is a case's Claude configuration overlays: a seeded case overlays the
// seed's settings.json and .claude.json, and a fresh case writes only its settings.
type driverClaudeConfig struct {
	fresh    bool
	settings string
	global   string
}

var (
	driverSeeded              = driverClaudeConfig{settings: "{}", global: "{}"}
	driverAutoDefaultSettings = driverClaudeConfig{settings: `{"permissions":{"defaultMode":"default"}}`, global: "{}"}
	driverEnvSetupGlobal      = driverClaudeConfig{settings: "{}", global: `{"numStartups":5,"autoModeEnvSetup":{"denials":5},"hasSeenAutoModeEntryWarning":true}`}
	driverEditReadAllowed     = driverClaudeConfig{settings: `{"permissions":{"allow":["Read(/{outside}/**)"]}}`, global: "{}"}
	driverNoTipSettings       = driverClaudeConfig{settings: `{"permissions":{"defaultMode":"default","disableAutoMode":"disable"}}`, global: "{}"}
	driverPanelSettings       = driverClaudeConfig{settings: `{"permissions":{"allow":["Edit(/{cwd}/panel-probe.txt)"]}}`, global: "{}"}
)

func (c driverClaudeConfig) overlays() (settings, global string) {
	if c.fresh {
		return c.settings, "{}"
	}
	return c.settings, c.global
}

type driverNudgeChoice uint8

const (
	driverNudgeAutoDefaultDecline driverNudgeChoice = iota + 1
	driverNudgeEnvSetupNotNow
	driverNudgeEnvSetupDontShowAgain
)

type driverChangedTo uint8

const (
	driverChangedToTrue driverChangedTo = iota + 1
	driverChangedToSet
)

type driverDocumentedChange struct {
	keyPath []string
	value   driverChangedTo
}

func (n driverNudgeChoice) stageHex() []string {
	switch n {
	case driverNudgeAutoDefaultDecline:
		return []string{"0e", "0d"}
	case driverNudgeEnvSetupNotNow:
		return []string{"32"}
	case driverNudgeEnvSetupDontShowAgain:
		return []string{"33"}
	default:
		panic("invalid driverNudgeChoice")
	}
}

func (n driverNudgeChoice) documentedChange() driverDocumentedChange {
	switch n {
	case driverNudgeAutoDefaultDecline:
		return driverDocumentedChange{keyPath: []string{".claude.json", "hasSeenAutoDefaultNudge"}, value: driverChangedToTrue}
	case driverNudgeEnvSetupNotNow:
		return driverDocumentedChange{keyPath: []string{".claude.json", "autoModeEnvSetup", "dismissedAt"}, value: driverChangedToSet}
	case driverNudgeEnvSetupDontShowAgain:
		return driverDocumentedChange{keyPath: []string{".claude.json", "autoModeEnvSetup", "dismissed"}, value: driverChangedToTrue}
	default:
		panic("invalid driverNudgeChoice")
	}
}

type driverApprovedCase struct {
	id             string
	backend        codingAgent
	permissionMode string
	mode           driverMode
	decision       opt[string]
	prefixHex      []string
	stageHex       []string
	probeArgument  opt[string]
	fileProbe      opt[driverFileProbe]
	database       string
	socket         string
	claudeConfig   opt[driverClaudeConfig]
	nudge          opt[driverNudgeChoice]
	baseline       opt[string]
	panelProbe     bool
	batched        bool
	size           terminalSize
}

var (
	driverFullSize      = terminalSize{rows: 36, cols: 120}
	driverPanelSize     = terminalSize{rows: 36, cols: 166}
	driverPanelMinSize  = terminalSize{rows: 36, cols: 144}
	driverWrappedSize   = terminalSize{rows: 36, cols: 52}
	driverBoxedWrapSize = terminalSize{rows: 36, cols: 89}
	driverNarrowSize    = terminalSize{rows: 36, cols: 57}
	driverCompactSize   = terminalSize{rows: 36, cols: 55}
	driverMinSize       = terminalSize{rows: 36, cols: 36}
	// The narrowest width at which the pi gate form's key-hint row stays on one row.
	driverPiMinSize    = terminalSize{rows: 36, cols: 49}
	driverHerdrEnvSize = terminalSize{rows: 36, cols: 166}
)

// tmux sets its own TERM and TERM_PROGRAM in every pane, so the herdr terminal identity reaches the
// native child only as spawn overrides; the values are those the live herdr child carried.
var driverHerdrChildEnvironment = [][2]string{
	{"TERM", "xterm-256color"},
	{"COLORTERM", "truecolor"},
	{"TERM_PROGRAM", "herdr"},
	{"TERM_PROGRAM_VERSION", "0.9.5"},
	{"HERDR_ENV", "1"},
}

func driverOpencodeSurvey(id, database, socket string, cols uint16) driverApprovedCase {
	return driverOpencodeNarrow(id, none[string](), nil, nil, some("allow"), database, socket, terminalSize{rows: 36, cols: cols})
}

func driverModeFor(decision opt[string]) driverMode {
	if decision.set {
		return driverModeAnswerStage
	}
	return driverModeNoAnswer
}

func driverOpencodeNarrow(id string, decision opt[string], prefixHex, stageHex []string, probeArgument opt[string], database, socket string, size terminalSize) driverApprovedCase {
	return driverApprovedCase{
		id:             id,
		backend:        codingAgentOpencode,
		permissionMode: "cafleet-ask",
		mode:           driverModeFor(decision),
		decision:       decision,
		prefixHex:      prefixHex,
		stageHex:       stageHex,
		probeArgument:  probeArgument,
		fileProbe:      none[driverFileProbe](),
		database:       database,
		socket:         socket,
		claudeConfig:   none[driverClaudeConfig](),
		nudge:          none[driverNudgeChoice](),
		baseline:       none[string](),
		panelProbe:     false,
		batched:        false,
		size:           size,
	}
}

func driverPiCase(id string, decision opt[string], prefixHex, stageHex []string, database, socket string, size terminalSize) driverApprovedCase {
	return driverApprovedCase{
		id:             id,
		backend:        codingAgentPi,
		permissionMode: "cafleet-ask",
		mode:           driverModeFor(decision),
		decision:       decision,
		prefixHex:      prefixHex,
		stageHex:       stageHex,
		probeArgument:  decision,
		fileProbe:      none[driverFileProbe](),
		database:       database,
		socket:         socket,
		claudeConfig:   none[driverClaudeConfig](),
		nudge:          none[driverNudgeChoice](),
		baseline:       none[string](),
		panelProbe:     false,
		batched:        false,
		size:           size,
	}
}

func driverClaudeCase(id, permissionMode string, config driverClaudeConfig, database, socket string) driverApprovedCase {
	return driverApprovedCase{
		id:             id,
		backend:        codingAgentClaude,
		permissionMode: permissionMode,
		mode:           driverModeNoAnswer,
		decision:       none[string](),
		prefixHex:      nil,
		stageHex:       nil,
		probeArgument:  none[string](),
		fileProbe:      none[driverFileProbe](),
		database:       database,
		socket:         socket,
		claudeConfig:   some(config),
		nudge:          none[driverNudgeChoice](),
		baseline:       none[string](),
		panelProbe:     false,
		batched:        false,
		size:           driverFullSize,
	}
}

func (c driverApprovedCase) withFile(probe driverFileProbe) driverApprovedCase {
	c.fileProbe = some(probe)
	return c
}

func (c driverApprovedCase) withBash(argument string) driverApprovedCase {
	c.probeArgument = some(argument)
	return c
}

func (c driverApprovedCase) withBatched() driverApprovedCase {
	c.batched = true
	return c
}

func (c driverApprovedCase) withPanel(size terminalSize) driverApprovedCase {
	c.panelProbe = true
	c.size = size
	return c
}

func (c driverApprovedCase) withAnswer(decision string, stageHex []string, baseline string) driverApprovedCase {
	c.mode = driverModeAnswerStage
	c.decision = some(decision)
	c.stageHex = stageHex
	c.baseline = some(baseline)
	return c
}

func (c driverApprovedCase) withChoice(nudge driverNudgeChoice, baseline string) driverApprovedCase {
	c.mode = driverModeAnswerStage
	c.stageHex = nudge.stageHex()
	c.nudge = some(nudge)
	c.baseline = some(baseline)
	return c
}

// driverHexBytes is stage split into its two-digit byte codes, as Rust's chunks(2) of its bytes.
func driverHexBytes(stage string) []string {
	var codes []string
	for index := 0; index < len(stage); index += 2 {
		codes = append(codes, stage[index:min(index+2, len(stage))])
	}
	return codes
}

func (c *driverApprovedCase) validate() error {
	containsEscape := slices.ContainsFunc(slices.Concat(c.prefixHex, c.stageHex), func(stage string) bool {
		return slices.Contains(driverHexBytes(stage), "1b")
	})
	if err := ensure((c.mode == driverModeNoAnswer) == (len(c.prefixHex) == 0 && len(c.stageHex) == 0) && !containsEscape,
		"reviewed case stages differ from its driver mode or contain Escape"); err != nil {
		return err
	}
	if c.fileProbe.set {
		switch c.permissionMode {
		case "auto":
			persistent := slices.ContainsFunc(c.stageHex, func(stage string) bool {
				return slices.ContainsFunc(driverHexBytes(stage), func(code string) bool {
					return slices.Contains([]string{"31", "32", "0d", "0a", "09"}, code)
				})
			})
			if err := ensure(!persistent, "a file-tool case stage selects a persistent choice, Enter or Tab"); err != nil {
				return err
			}
		case "default":
			if err := ensure(!slices.ContainsFunc(c.stageHex, func(stage string) bool { return stage != "31" && stage != "33" }),
				"a default-mode file-tool case may send only 31 or 33"); err != nil {
				return err
			}
		default:
			return appError("a file-tool case runs in auto or default mode")
		}
	}
	if nudge, set := c.nudge.get(); set {
		if err := ensure(slices.Equal(c.stageHex, nudge.stageHex()) && len(c.prefixHex) == 0 && !c.decision.set && !c.fileProbe.set && !c.probeArgument.set,
			"a nudge case sends exactly its dismissal stages"); err != nil {
			return err
		}
	}
	probe, probed := c.fileProbe.get()
	twoProbeCalls := (probed && probe.kind == driverProbeRead && !c.probeArgument.set) || (!probed && c.probeArgument.set)
	if err := ensure(!c.batched || (c.claudeConfig.set && c.permissionMode == "default" && !c.panelProbe && twoProbeCalls),
		"a batched case issues two Read or two Bash probe calls in a configured default-mode case"); err != nil {
		return err
	}
	config, configured := c.claudeConfig.get()
	if !configured {
		return ensure(!c.fileProbe.set && !c.nudge.set && !c.baseline.set && !c.panelProbe,
			"file-tool, nudge, baseline and sidebar cases own a Claude configuration")
	}
	if err := ensure(c.backend == codingAgentClaude, "a configured case runs Claude"); err != nil {
		return err
	}
	settings, global := config.overlays()
	outside := c.outsideDirectory()
	cwd := filepath.Join(driverNeutralRoot, c.id)
	if _, err := driverSettingsOverlay(settings, cwd, outside); err != nil {
		return err
	}
	if _, err := driverOverlay(global, cwd, outside); err != nil {
		return err
	}
	if err := ensure(!config.fresh || c.mode == driverModeNoAnswer, "a fresh onboarding case answers nothing"); err != nil {
		return err
	}
	baselineID, named := c.baseline.get()
	switch answers := c.mode == driverModeAnswerStage; {
	case answers && named:
		index := slices.IndexFunc(driverApprovedCases[:], func(candidate driverApprovedCase) bool { return candidate.id == baselineID })
		if index < 0 {
			return appError("a baseline names an unreviewed case")
		}
		baseline := &driverApprovedCases[index]
		if err := ensure(baseline.mode == driverModeNoAnswer && baseline.claudeConfig == c.claudeConfig && baseline.permissionMode == c.permissionMode,
			"a baseline is a no-answer case with the same configuration and mode"); err != nil {
			return err
		}
	case !answers && !named:
	default:
		return appError("exactly every configured answer case names its baseline")
	}
	return ensure(!c.panelProbe || (!c.fileProbe.set && c.probeArgument.set && (c.size == driverPanelSize || c.size == driverPanelMinSize)),
		"a sidebar case runs the Bash probe at a sidebar size")
}

func driverSelectApproved(owned *driverCase) (*driverApprovedCase, error) {
	index := slices.IndexFunc(driverApprovedCases[:], func(approved driverApprovedCase) bool { return approved.id == owned.CaseID })
	if index < 0 {
		return nil, appError("unreviewed native case")
	}
	approved := &driverApprovedCases[index]
	if err := approved.validate(); err != nil {
		return nil, err
	}
	var mode string
	var window uint64
	switch approved.mode {
	case driverModeNoAnswer:
		mode, window = "no_answer", 0
	case driverModeAnswerStage:
		mode, window = "answer_stage", driverAnswerWindowMS
	default:
		panic("invalid driverMode")
	}
	if err := ensure(owned.Mode == mode && owned.AnswerWindowMS == window && owned.Decision == approved.decision &&
		slices.Equal(owned.PrefixHex, approved.prefixHex) && slices.Equal(owned.StageHex, approved.stageHex) && owned.ProbeArgument == approved.probeArgument,
		"case decision, stage or probe binding differs from its reviewed case"); err != nil {
		return nil, err
	}
	return approved, nil
}

func (c *driverApprovedCase) childEnvironment() [][2]string {
	var environment [][2]string
	if c.id == "opencode-herdr-env-cells-001" || (c.backend == codingAgentPi && strings.HasPrefix(c.id, "pi-herdr-")) {
		environment = slices.Clone(driverHerdrChildEnvironment)
	}
	if directory, set := c.claudeConfigDirectory().get(); set {
		environment = append(environment, [2]string{"CLAUDE_CONFIG_DIR", directory})
	}
	return environment
}

func (c *driverApprovedCase) outsideDirectory() opt[string] {
	if !c.fileProbe.set {
		return none[string]()
	}
	return some(filepath.Join(driverNeutralRoot, c.id+"-outside"))
}

func (c *driverApprovedCase) claudeConfigDirectory() opt[string] {
	if !c.claudeConfig.set {
		return none[string]()
	}
	return some(filepath.Join(driverPrivateRoot(), c.id+"-claude"))
}

func (c *driverApprovedCase) requestedMode() opt[string] {
	policy, err := c.backend.resolvePolicy(none[string]())
	if err != nil {
		panic("an omitted launch policy always resolves")
	}
	if c.permissionMode == policy.resolvedMode {
		return none[string]()
	}
	return some(c.permissionMode)
}

func (c *driverApprovedCase) memberName() string {
	return "cafleet-194-" + c.id
}

func (c *driverApprovedCase) tmuxGeometry() string {
	return fmt.Sprintf("%d|%d", c.size.cols, c.size.rows)
}

func (c *driverApprovedCase) requestedMarker() opt[string] {
	argument, set := c.probeArgument.get()
	if !set {
		return none[string]()
	}
	label, _, _ := strings.Cut(argument, " ")
	return some(label + ".executed")
}

// driverDisplayJoin is Rust's base.join(name).display(): an empty name leaves a trailing separator.
func driverDisplayJoin(base, name string) string {
	if name == "" {
		return strings.TrimSuffix(base, "/") + "/"
	}
	return filepath.Join(base, name)
}

func (c *driverApprovedCase) prompt(cwd string, helper opt[string]) string {
	probe, probed := c.fileProbe.get()
	outside, hasOutside := c.outsideDirectory().get()
	if probed && probe.kind == driverProbeBashTouch && hasOutside {
		return fmt.Sprintf("Use the %s tool to %s %s as one command. Preserve any native permission request for the operator. After the native decision, report the actual result and stop. Keep work inside this disposable operation.",
			probe.tool(), probe.operation(), driverDisplayJoin(outside, probe.target()))
	}
	if c.batched && probed && hasOutside {
		return fmt.Sprintf("In one turn, issue two parallel %s tool calls: one to %s on exactly %s and one to %s on exactly %s. Preserve every native permission request for the operator. After the native decisions, report the actual results and stop. Keep work inside this disposable operation.",
			probe.tool(), probe.operation(), driverDisplayJoin(outside, probe.target()), probe.operation(), driverDisplayJoin(outside, driverBatchedReadProbe))
	}
	if probed && hasOutside {
		return fmt.Sprintf("Use the %s tool to %s on exactly %s. Preserve any native permission request for the operator. After the native decision, report the actual result and stop. Keep work inside this disposable operation.",
			probe.tool(), probe.operation(), driverDisplayJoin(outside, probe.target()))
	}
	helperPath, bound := helper.get()
	if !bound {
		panic("a shell-probe case binds its helper")
	}
	argument, hasArgument := c.probeArgument.get()
	if c.batched && hasArgument {
		return fmt.Sprintf("In one turn, issue two parallel Bash tool calls: one attempting exactly %[1]s %[2]s first and one attempting exactly %[1]s %[2]s second, each as one command. Preserve every native permission request for the operator. After the native decisions, report the actual results and stop. Keep work inside this disposable operation.",
			helperPath, argument)
	}
	if c.panelProbe && hasArgument {
		return fmt.Sprintf("Use the Edit tool to replace the text before with after in the file %s. Then use Bash to attempt exactly %s %s as one command. Preserve any native permission request for the operator. After the native decision, report the actual result and stop. Keep work inside this disposable operation.",
			driverDisplayJoin(cwd, driverPanelProbe), helperPath, argument)
	}
	if !hasArgument {
		return driverToolFreePrompt
	}
	switch c.backend {
	case codingAgentCodex:
		return fmt.Sprintf("Use the shell tool to run exactly %s %s as one command with sandbox_permissions set to require_escalated and the justification \"CAFleet permission probe\". Preserve any native approval request for the operator. After the native decision, report the actual result and stop. Keep work inside this disposable operation.",
			helperPath, argument)
	case codingAgentOpencode:
		return fmt.Sprintf("Use the shell tool to run exactly %s %s as one command. Preserve any native permission request for the operator. After the native decision, report the actual result and stop. Keep work inside this disposable operation.",
			helperPath, argument)
	case codingAgentClaude:
		return fmt.Sprintf("Use Bash to attempt exactly %s %s as one command. Preserve any native permission request for the operator. After the native decision, report the actual result and stop. Keep work inside this disposable operation.",
			helperPath, argument)
	case codingAgentHermes:
		return fmt.Sprintf("Use the terminal tool to run exactly %s %s as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result and stop. Keep work inside this disposable operation.",
			helperPath, argument)
	case codingAgentPi:
		// The relative helper keeps the gate form's operation row on one row at the
		// profile's narrowest width; pi runs bash in the case directory.
		name := filepath.Base(helperPath)
		if name == "/" || name == "." || name == ".." {
			panic("the helper is a file in the case directory")
		}
		return fmt.Sprintf("Use the bash tool to run exactly ./%s %s as one command. Preserve any native permission request for the operator. After the native decision, report the actual result and stop. Keep work inside this disposable operation.",
			name, argument)
	default:
		panic("invalid codingAgent")
	}
}

func (c *driverApprovedCase) forbiddenMarkers() []string {
	var markers []string
	for _, marker := range []string{"allow.executed", "deny.executed"} {
		if marker == "allow.executed" && c.decision == some("allow") {
			continue
		}
		markers = append(markers, marker)
	}
	return markers
}

var driverApprovedCases = [...]driverApprovedCase{
	{
		id: "cells-002", backend: codingAgentClaude, permissionMode: "default", mode: driverModeNoAnswer,
		decision: none[string](), prefixHex: nil, stageHex: nil, probeArgument: some("allow"), fileProbe: none[driverFileProbe](),
		database: "nc02.db", socket: "tc02", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverFullSize,
	},
	{
		id: "ans-allow-001", backend: codingAgentClaude, permissionMode: "default", mode: driverModeAnswerStage,
		decision: some("allow"), prefixHex: nil, stageHex: []string{"31"}, probeArgument: some("allow"), fileProbe: none[driverFileProbe](),
		database: "na01.db", socket: "ta01", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverFullSize,
	},
	{
		id: "ans-deny-001", backend: codingAgentClaude, permissionMode: "default", mode: driverModeAnswerStage,
		decision: some("deny"), prefixHex: nil, stageHex: []string{"34"}, probeArgument: some("deny"), fileProbe: none[driverFileProbe](),
		database: "nd01.db", socket: "td01", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverFullSize,
	},
	{
		id: "idle-001", backend: codingAgentClaude, permissionMode: "default", mode: driverModeNoAnswer,
		decision: none[string](), prefixHex: nil, stageHex: nil, probeArgument: none[string](), fileProbe: none[driverFileProbe](),
		database: "ni01.db", socket: "ti01", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverFullSize,
	},
	{
		id: "codex-trust-001", backend: codingAgentCodex, permissionMode: "on-request", mode: driverModeNoAnswer,
		decision: none[string](), prefixHex: nil, stageHex: nil, probeArgument: none[string](), fileProbe: none[driverFileProbe](),
		database: "xt01.db", socket: "xt01", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverFullSize,
	},
	{
		id: "codex-cells-001", backend: codingAgentCodex, permissionMode: "on-request", mode: driverModeNoAnswer,
		decision: none[string](), prefixHex: nil, stageHex: nil, probeArgument: some("allow"), fileProbe: none[driverFileProbe](),
		database: "xc01.db", socket: "xc01", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverFullSize,
	},
	{
		id: "codex-ans-allow-001", backend: codingAgentCodex, permissionMode: "on-request", mode: driverModeAnswerStage,
		decision: some("allow"), prefixHex: nil, stageHex: []string{"79"}, probeArgument: some("allow"), fileProbe: none[driverFileProbe](),
		database: "xa01.db", socket: "xa01", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverFullSize,
	},
	{
		id: "codex-ans-deny-001", backend: codingAgentCodex, permissionMode: "on-request", mode: driverModeAnswerStage,
		decision: some("deny"), prefixHex: nil, stageHex: []string{"33"}, probeArgument: some("deny"), fileProbe: none[driverFileProbe](),
		database: "xd01.db", socket: "xd01", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverFullSize,
	},
	{
		id: "codex-idle-001", backend: codingAgentCodex, permissionMode: "on-request", mode: driverModeNoAnswer,
		decision: none[string](), prefixHex: nil, stageHex: nil, probeArgument: none[string](), fileProbe: none[driverFileProbe](),
		database: "xi01.db", socket: "xi01", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverFullSize,
	},
	{
		id: "claude-wrap-cells-001", backend: codingAgentClaude, permissionMode: "default", mode: driverModeNoAnswer,
		decision: none[string](), prefixHex: nil, stageHex: nil, probeArgument: some("allow"), fileProbe: none[driverFileProbe](),
		database: "nwc1.db", socket: "twc1", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverWrappedSize,
	},
	{
		id: "claude-wrap-ans-allow-001", backend: codingAgentClaude, permissionMode: "default", mode: driverModeAnswerStage,
		decision: some("allow"), prefixHex: nil, stageHex: []string{"31"}, probeArgument: some("allow"), fileProbe: none[driverFileProbe](),
		database: "nwa1.db", socket: "twa1", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverWrappedSize,
	},
	{
		id: "claude-wrap-ans-deny-001", backend: codingAgentClaude, permissionMode: "default", mode: driverModeAnswerStage,
		decision: some("deny"), prefixHex: nil, stageHex: []string{"34"}, probeArgument: some("deny"), fileProbe: none[driverFileProbe](),
		database: "nwd1.db", socket: "twd1", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverWrappedSize,
	},
	{
		id: "claude-boxed-cells-089-wide-operation-probe-01", backend: codingAgentClaude, permissionMode: "default", mode: driverModeNoAnswer,
		decision: none[string](), prefixHex: nil, stageHex: nil, probeArgument: some("allow"), fileProbe: none[driverFileProbe](),
		database: "nb89.db", socket: "tb89", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverBoxedWrapSize,
	},
	{
		id: "claude-boxed-cells-120-wide-operation-probe-01", backend: codingAgentClaude, permissionMode: "default", mode: driverModeNoAnswer,
		decision: none[string](), prefixHex: nil, stageHex: nil, probeArgument: some("allow"), fileProbe: none[driverFileProbe](),
		database: "nb12.db", socket: "tb12", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverFullSize,
	},
	{
		id: "opencode-cells-001", backend: codingAgentOpencode, permissionMode: "cafleet-ask", mode: driverModeNoAnswer,
		decision: none[string](), prefixHex: nil, stageHex: nil, probeArgument: some("allow"), fileProbe: none[driverFileProbe](),
		database: "oc01.db", socket: "oc01", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverFullSize,
	},
	{
		id: "opencode-ans-allow-001", backend: codingAgentOpencode, permissionMode: "cafleet-ask", mode: driverModeAnswerStage,
		decision: some("allow"), prefixHex: nil, stageHex: []string{"0d"}, probeArgument: some("allow"), fileProbe: none[driverFileProbe](),
		database: "oa01.db", socket: "oa01", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverFullSize,
	},
	{
		id: "opencode-ans-deny-001", backend: codingAgentOpencode, permissionMode: "cafleet-ask", mode: driverModeAnswerStage,
		decision: some("deny"), prefixHex: nil, stageHex: []string{"68", "0d"}, probeArgument: some("deny"), fileProbe: none[driverFileProbe](),
		database: "od01.db", socket: "od01", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverFullSize,
	},
	{
		id: "opencode-ans-deny-002", backend: codingAgentOpencode, permissionMode: "cafleet-ask", mode: driverModeAnswerStage,
		decision: some("deny"), prefixHex: []string{"68"}, stageHex: []string{"0d", "0d"}, probeArgument: some("deny"), fileProbe: none[driverFileProbe](),
		database: "od02.db", socket: "od02", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverFullSize,
	},
	{
		id: "opencode-ans-deny-003", backend: codingAgentOpencode, permissionMode: "cafleet-ask", mode: driverModeAnswerStage,
		decision: some("deny"), prefixHex: []string{"68", "0d"}, stageHex: []string{"0d"}, probeArgument: some("deny"), fileProbe: none[driverFileProbe](),
		database: "od03.db", socket: "od03", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverFullSize,
	},
	{
		id: "opencode-focus-always-001", backend: codingAgentOpencode, permissionMode: "cafleet-ask", mode: driverModeAnswerStage,
		decision: some("deny"), prefixHex: []string{"6c"}, stageHex: nil, probeArgument: some("deny"), fileProbe: none[driverFileProbe](),
		database: "of01.db", socket: "of01", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverFullSize,
	},
	{
		id: "opencode-focus-always-002", backend: codingAgentOpencode, permissionMode: "cafleet-ask", mode: driverModeAnswerStage,
		decision: some("deny"), prefixHex: []string{"6c"}, stageHex: nil, probeArgument: some("deny"), fileProbe: none[driverFileProbe](),
		database: "of02.db", socket: "of02", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverFullSize,
	},
	{
		id: "opencode-idle-001", backend: codingAgentOpencode, permissionMode: "cafleet-ask", mode: driverModeNoAnswer,
		decision: none[string](), prefixHex: nil, stageHex: nil, probeArgument: none[string](), fileProbe: none[driverFileProbe](),
		database: "oi01.db", socket: "oi01", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverFullSize,
	},
	{
		id: "opencode-herdr-env-cells-001", backend: codingAgentOpencode, permissionMode: "cafleet-ask", mode: driverModeNoAnswer,
		decision: none[string](), prefixHex: nil, stageHex: nil, probeArgument: some("allow"), fileProbe: none[driverFileProbe](),
		database: "oh01.db", socket: "oh01", claudeConfig: none[driverClaudeConfig](), nudge: none[driverNudgeChoice](),
		baseline: none[string](), panelProbe: false, batched: false, size: driverHerdrEnvSize,
	},
	driverOpencodeSurvey("opencode-survey-096", "os096.db", "os096", 96),
	driverOpencodeSurvey("opencode-survey-080", "os080.db", "os080", 80),
	driverOpencodeSurvey("opencode-survey-064", "os064.db", "os064", 64),
	driverOpencodeSurvey("opencode-survey-048", "os048.db", "os048", 48),
	driverOpencodeSurvey("opencode-survey-040", "os040.db", "os040", 40),
	driverOpencodeSurvey("opencode-survey-076", "os076.db", "os076", 76),
	driverOpencodeSurvey("opencode-survey-072", "os072.db", "os072", 72),
	driverOpencodeSurvey("opencode-survey-068", "os068.db", "os068", 68),
	driverOpencodeSurvey("opencode-survey-060", "os060.db", "os060", 60),
	driverOpencodeSurvey("opencode-survey-056", "os056.db", "os056", 56),
	driverOpencodeSurvey("opencode-survey-052", "os052.db", "os052", 52),
	driverOpencodeSurvey("opencode-survey-032", "os032.db", "os032", 32),
	driverOpencodeSurvey("opencode-survey-024", "os024.db", "os024", 24),
	driverOpencodeSurvey("opencode-survey-079", "os079.db", "os079", 79),
	driverOpencodeSurvey("opencode-survey-078", "os078.db", "os078", 78),
	driverOpencodeSurvey("opencode-survey-077", "os077.db", "os077", 77),
	driverOpencodeSurvey("opencode-survey-055", "os055.db", "os055", 55),
	driverOpencodeSurvey("opencode-survey-054", "os054.db", "os054", 54),
	driverOpencodeSurvey("opencode-survey-053", "os053.db", "os053", 53),
	driverOpencodeSurvey("opencode-survey-039", "os039.db", "os039", 39),
	driverOpencodeSurvey("opencode-survey-038", "os038.db", "os038", 38),
	driverOpencodeSurvey("opencode-survey-037", "os037.db", "os037", 37),
	driverOpencodeSurvey("opencode-survey-036", "os036.db", "os036", 36),
	driverOpencodeSurvey("opencode-survey-035", "os035.db", "os035", 35),
	driverOpencodeSurvey("opencode-survey-034", "os034.db", "os034", 34),
	driverOpencodeSurvey("opencode-survey-033", "os033.db", "os033", 33),
	driverOpencodeNarrow("opencode-narrow-cells-057", none[string](), nil, nil, some("allow"), "opencode-narrow-cells-057.db", "opencode-narrow-cells-057", driverNarrowSize),
	driverOpencodeNarrow("opencode-narrow-allow-057", some("allow"), nil, []string{"0d"}, some("allow"), "opencode-narrow-allow-057.db", "opencode-narrow-allow-057", driverNarrowSize),
	driverOpencodeNarrow("opencode-narrow-deny-002-057", some("deny"), []string{"68"}, []string{"0d", "0d"}, some("deny"), "opencode-narrow-deny-002-057.db", "opencode-narrow-deny-002-057", driverNarrowSize),
	driverOpencodeNarrow("opencode-narrow-deny-003-057", some("deny"), []string{"68", "0d"}, []string{"0d"}, some("deny"), "opencode-narrow-deny-003-057.db", "opencode-narrow-deny-003-057", driverNarrowSize),
	driverOpencodeNarrow("opencode-narrow-long-057", none[string](), nil, nil, some(driverLongCommandArgument), "opencode-narrow-long-057.db", "opencode-narrow-long-057", driverNarrowSize),
	driverOpencodeNarrow("opencode-narrow-idle-057", none[string](), nil, nil, none[string](), "opencode-narrow-idle-057.db", "opencode-narrow-idle-057", driverNarrowSize),
	driverOpencodeNarrow("opencode-compact-cells-055", none[string](), nil, nil, some("allow"), "opencode-compact-cells-055.db", "opencode-compact-cells-055", driverCompactSize),
	driverOpencodeNarrow("opencode-compact-deny-002-055", some("deny"), []string{"68"}, []string{"0d", "0d"}, some("deny"), "opencode-compact-deny-002-055.db", "opencode-compact-deny-002-055", driverCompactSize),
	driverOpencodeNarrow("opencode-compact-deny-003-055", some("deny"), []string{"68", "0d"}, []string{"0d"}, some("deny"), "opencode-compact-deny-003-055.db", "opencode-compact-deny-003-055", driverCompactSize),
	driverOpencodeNarrow("opencode-narrow-cells-min", none[string](), nil, nil, some("allow"), "opencode-narrow-cells-min.db", "opencode-narrow-cells-min", driverMinSize),
	driverOpencodeNarrow("opencode-narrow-deny-002-min", some("deny"), []string{"68"}, []string{"0d", "0d"}, some("deny"), "opencode-narrow-deny-002-min.db", "opencode-narrow-deny-002-min", driverMinSize),
	driverOpencodeNarrow("opencode-narrow-deny-003-min", some("deny"), []string{"68", "0d"}, []string{"0d"}, some("deny"), "opencode-narrow-deny-003-min.db", "opencode-narrow-deny-003-min", driverMinSize),
	driverClaudeCase("claude-read-outside-cells-001", "auto", driverSeeded, "rc01.db", "rc01").withFile(driverReadFileProbe),
	driverClaudeCase("claude-read-outside-long-001", "auto", driverSeeded, "rl01.db", "rl01").withFile(driverFileProbe{kind: driverProbeRead, readProbe: driverLongReadProbe}),
	driverClaudeCase("claude-read-outside-allow-001", "auto", driverSeeded, "ra01.db", "ra01").withFile(driverReadFileProbe).withAnswer("allow", []string{"34"}, "claude-read-outside-cells-001"),
	driverClaudeCase("claude-read-outside-deny-001", "auto", driverSeeded, "rd01.db", "rd01").withFile(driverReadFileProbe).withAnswer("deny", []string{"33"}, "claude-read-outside-cells-001"),
	driverClaudeCase("claude-edit-outside-cells-001", "auto", driverSeeded, "ec01.db", "ec01").withFile(driverFileProbe{kind: driverProbeEdit}),
	driverClaudeCase("claude-write-outside-cells-001", "auto", driverSeeded, "wc01.db", "wc01").withFile(driverFileProbe{kind: driverProbeWrite}),
	driverClaudeCase("claude-glob-outside-cells-001", "auto", driverSeeded, "gc01.db", "gc01").withFile(driverFileProbe{kind: driverProbeGlob}),
	driverClaudeCase("claude-grep-outside-cells-001", "auto", driverSeeded, "sc01.db", "sc01").withFile(driverFileProbe{kind: driverProbeGrep}),
	driverClaudeCase("claude-auto-default-cells-001", "auto", driverAutoDefaultSettings, "dc01.db", "dc01"),
	driverClaudeCase("claude-auto-default-decline-001", "auto", driverAutoDefaultSettings, "dd01.db", "dd01").withChoice(driverNudgeAutoDefaultDecline, "claude-auto-default-cells-001"),
	driverClaudeCase("claude-auto-default-accept-edits-cells-001", "auto", driverClaudeConfig{settings: `{"permissions":{"defaultMode":"acceptEdits"}}`, global: "{}"}, "de01.db", "de01"),
	driverClaudeCase("claude-question-pending-cells-001", "auto", driverAutoDefaultSettings, "qp01.db", "qp01"),
	driverClaudeCase("claude-question-pending-cells-002", "auto", driverAutoDefaultSettings, "qp02.db", "qp02"),
	driverClaudeCase("claude-auto-env-setup-cells-001", "auto", driverEnvSetupGlobal, "bc01.db", "bc01"),
	driverClaudeCase("claude-auto-env-setup-not-now-001", "auto", driverEnvSetupGlobal, "bn01.db", "bn01").withChoice(driverNudgeEnvSetupNotNow, "claude-auto-env-setup-cells-001"),
	driverClaudeCase("claude-auto-env-setup-dont-show-again-001", "auto", driverEnvSetupGlobal, "bs01.db", "bs01").withChoice(driverNudgeEnvSetupDontShowAgain, "claude-auto-env-setup-cells-001"),
	driverClaudeCase("claude-primary-auto-tip-cells-001", "default", driverClaudeConfig{fresh: true, settings: "{}"}, "pt01.db", "pt01").withBash("allow"),
	driverClaudeCase("claude-primary-auto-default-cells-001", "auto", driverClaudeConfig{fresh: true, settings: `{"permissions":{"defaultMode":"default"}}`}, "pd01.db", "pd01"),
	driverClaudeCase("claude-read-file-cells-001", "default", driverSeeded, "frc1.db", "frc1").withFile(driverReadFileProbe),
	driverClaudeCase("claude-read-file-allow-001", "default", driverSeeded, "fra1.db", "fra1").withFile(driverReadFileProbe).withAnswer("allow", []string{"31"}, "claude-read-file-cells-001"),
	driverClaudeCase("claude-read-file-deny-001", "default", driverSeeded, "frd1.db", "frd1").withFile(driverReadFileProbe).withAnswer("deny", []string{"33"}, "claude-read-file-cells-001"),
	driverClaudeCase("claude-edit-file-cells-001", "default", driverEditReadAllowed, "fec1.db", "fec1").withFile(driverFileProbe{kind: driverProbeEdit}),
	driverClaudeCase("claude-edit-file-allow-001", "default", driverEditReadAllowed, "fea1.db", "fea1").withFile(driverFileProbe{kind: driverProbeEdit}).withAnswer("allow", []string{"31"}, "claude-edit-file-cells-001"),
	driverClaudeCase("claude-edit-file-deny-001", "default", driverEditReadAllowed, "fed1.db", "fed1").withFile(driverFileProbe{kind: driverProbeEdit}).withAnswer("deny", []string{"33"}, "claude-edit-file-cells-001"),
	driverClaudeCase("claude-create-file-cells-001", "default", driverSeeded, "fcc1.db", "fcc1").withFile(driverFileProbe{kind: driverProbeWrite}),
	driverClaudeCase("claude-create-file-allow-001", "default", driverSeeded, "fca1.db", "fca1").withFile(driverFileProbe{kind: driverProbeWrite}).withAnswer("allow", []string{"31"}, "claude-create-file-cells-001"),
	driverClaudeCase("claude-create-file-deny-001", "default", driverSeeded, "fcd1.db", "fcd1").withFile(driverFileProbe{kind: driverProbeWrite}).withAnswer("deny", []string{"33"}, "claude-create-file-cells-001"),
	driverClaudeCase("claude-bash-no-tip-cells-001", "default", driverNoTipSettings, "ntc1.db", "ntc1").withBash("allow"),
	driverClaudeCase("claude-bash-no-tip-allow-001", "default", driverNoTipSettings, "nta1.db", "nta1").withBash("allow").withAnswer("allow", []string{"31"}, "claude-bash-no-tip-cells-001"),
	driverClaudeCase("claude-bash-no-tip-deny-001", "default", driverNoTipSettings, "ntd1.db", "ntd1").withBash("deny").withAnswer("deny", []string{"33"}, "claude-bash-no-tip-cells-001"),
	driverClaudeCase("claude-bash-no-tip-access-cells-001", "default", driverNoTipSettings, "nac1.db", "nac1").withFile(driverFileProbe{kind: driverProbeBashTouch}),
	driverClaudeCase("claude-read-file-batched-cells-001", "default", driverSeeded, "rbc1.db", "rbc1").withFile(driverReadFileProbe).withBatched().withAnswer("allow", []string{"31"}, "claude-read-file-cells-001"),
	driverClaudeCase("claude-read-file-batched-deny-001", "default", driverSeeded, "rbd1.db", "rbd1").withFile(driverReadFileProbe).withBatched().withAnswer("deny", []string{"33"}, "claude-read-file-cells-001"),
	driverClaudeCase("claude-bash-batched-cells-001", "default", driverSeeded, "bbc1.db", "bbc1").withBash("allow").withBatched(),
	driverClaudeCase("claude-panel-auto-tip-cells-001", "default", driverPanelSettings, "pc01.db", "pc01").withBash("allow").withPanel(driverPanelSize),
	driverClaudeCase("claude-panel-auto-tip-cells-002", "default", driverPanelSettings, "pc02.db", "pc02").withBash("allow").withPanel(driverPanelMinSize),
	driverClaudeCase("claude-panel-auto-tip-allow-001", "default", driverPanelSettings, "pa01.db", "pa01").withBash("allow").withPanel(driverPanelSize).withAnswer("allow", []string{"31"}, "claude-panel-auto-tip-cells-001"),
	driverPiCase("pi-allow-120", some("allow"), nil, []string{"0d"}, "pa120.db", "pa120", driverFullSize),
	driverPiCase("pi-deny-001-120", some("deny"), nil, []string{"6a", "0d"}, "pd1120.db", "pd1120", driverFullSize),
	driverPiCase("pi-deny-002-120", some("deny"), []string{"6a"}, []string{"0d"}, "pd2120.db", "pd2120", driverFullSize),
	driverPiCase("pi-prompt-120", none[string](), nil, nil, "pp120.db", "pp120", driverFullSize),
	driverPiCase("pi-exec-120", none[string](), nil, nil, "pe120.db", "pe120", driverFullSize),
	driverPiCase("pi-allow-049", some("allow"), nil, []string{"0d"}, "pa049.db", "pa049", driverPiMinSize),
	driverPiCase("pi-deny-001-049", some("deny"), nil, []string{"6a", "0d"}, "pd1049.db", "pd1049", driverPiMinSize),
	driverPiCase("pi-deny-002-049", some("deny"), []string{"6a"}, []string{"0d"}, "pd2049.db", "pd2049", driverPiMinSize),
	driverPiCase("pi-prompt-049", none[string](), nil, nil, "pp049.db", "pp049", driverPiMinSize),
	driverPiCase("pi-exec-049", none[string](), nil, nil, "pe049.db", "pe049", driverPiMinSize),
	driverPiCase("pi-herdr-allow-120", some("allow"), nil, []string{"0d"}, "pha120.db", "pha120", driverFullSize),
	driverPiCase("pi-herdr-deny-001-120", some("deny"), nil, []string{"6a", "0d"}, "phd1120.db", "phd1120", driverFullSize),
	driverPiCase("pi-herdr-deny-002-120", some("deny"), []string{"6a"}, []string{"0d"}, "phd2120.db", "phd2120", driverFullSize),
	driverPiCase("pi-herdr-prompt-120", none[string](), nil, nil, "php120.db", "php120", driverFullSize),
	driverPiCase("pi-herdr-exec-120", none[string](), nil, nil, "phe120.db", "phe120", driverFullSize),
	driverPiCase("pi-herdr-allow-049", some("allow"), nil, []string{"0d"}, "pha049.db", "pha049", driverPiMinSize),
	driverPiCase("pi-herdr-deny-001-049", some("deny"), nil, []string{"6a", "0d"}, "phd1049.db", "phd1049", driverPiMinSize),
	driverPiCase("pi-herdr-deny-002-049", some("deny"), []string{"6a"}, []string{"0d"}, "phd2049.db", "phd2049", driverPiMinSize),
	driverPiCase("pi-herdr-prompt-049", none[string](), nil, nil, "php049.db", "php049", driverPiMinSize),
	driverPiCase("pi-herdr-exec-049", none[string](), nil, nil, "phe049.db", "phe049", driverPiMinSize),
}

// The reviewed table holds exactly 118 cases; any other count fails to compile.
var _ = [1]struct{}{}[len(driverApprovedCases)-118]

type driverReference struct {
	Path   string `json:"path"`
	Sha256 string `json:"sha256"`
}

func (r driverReference) verify() error {
	info, err := os.Lstat(r.Path)
	if err != nil {
		return ioError(ioErrorText(err))
	}
	if err := ensure(filepath.IsAbs(r.Path) && info.Mode().IsRegular() && info.Size() <= 512*1024*1024, "bounded absolute evidence file required"); err != nil {
		return err
	}
	digest, err := driverFileDigest(r.Path)
	if err != nil {
		return err
	}
	return ensure(digest == r.Sha256, "driver evidence digest changed")
}

type driverTmux struct {
	SocketPath   string `json:"socket_path"`
	SocketDevice uint64 `json:"socket_device"`
	SocketInode  uint64 `json:"socket_inode"`
	ServerPID    int64  `json:"server_pid"`
	Session      string `json:"session"`
	WindowID     string `json:"window_id"`
	PaneID       string `json:"pane_id"`
}

type driverCase struct {
	SchemaVersion           uint32               `json:"schema_version"`
	CaseID                  string               `json:"case_id"`
	Mode                    string               `json:"mode"`
	AnswerWindowMS          uint64               `json:"answer_window_ms"`
	Decision                opt[string]          `json:"decision"`
	PrefixHex               []string             `json:"prefix_hex"`
	StageHex                []string             `json:"stage_hex"`
	ProbeArgument           opt[string]          `json:"probe_argument"`
	CaseNonce               string               `json:"case_nonce"`
	ExpiresAtMonoNS         uint64               `json:"expires_at_mono_ns"`
	CleanupReserveMS        uint64               `json:"cleanup_reserve_ms"`
	DatabaseURL             string               `json:"database_url"`
	ConfigPath              string               `json:"config_path"`
	Cwd                     string               `json:"cwd"`
	OutputDirectory         string               `json:"output_directory"`
	ArmPath                 string               `json:"arm_path"`
	ProducerManifestPath    string               `json:"producer_manifest_path"`
	LaunchEvidencePath      string               `json:"launch_evidence_path"`
	DriverOwnershipPath     string               `json:"driver_ownership_path"`
	StartedRecordPath       string               `json:"started_record_path"`
	StopPath                string               `json:"stop_path"`
	PrivacyConfirmationPath string               `json:"privacy_confirmation_path"`
	ResultPath              string               `json:"result_path"`
	DescriptorCopyPath      string               `json:"descriptor_copy_path"`
	NativeVersion           string               `json:"native_version"`
	DirectorBackend         string               `json:"director_backend"`
	NativeExecutable        driverReference      `json:"native_executable"`
	Helper                  opt[driverReference] `json:"helper"`
	Libtest                 driverReference      `json:"libtest"`
	SourceManifest          driverReference      `json:"source_manifest"`
	VersionEvidence         driverReference      `json:"version_evidence"`
	EnvironmentEvidence     driverReference      `json:"environment_evidence"`
	CapturePolicy           driverReference      `json:"capture_policy"`
	Tmux                    driverTmux           `json:"tmux"`
}

type driverOwnership struct {
	SchemaVersion      uint32          `json:"schema_version"`
	CaseManifestSha256 string          `json:"case_manifest_sha256"`
	CaseNonce          string          `json:"case_nonce"`
	Driver             processIdentity `json:"driver"`
	Server             processIdentity `json:"server"`
	Tmux               driverTmux      `json:"tmux"`
	ExpiresAtMonoNS    uint64          `json:"expires_at_mono_ns"`
}

type driverStarted struct {
	SchemaVersion         uint32          `json:"schema_version"`
	CaseManifestSha256    string          `json:"case_manifest_sha256"`
	DriverOwnershipSha256 string          `json:"driver_ownership_sha256"`
	Identity              controlIdentity `json:"identity"`
	Runner                processIdentity `json:"runner"`
	Child                 processIdentity `json:"child"`
	PaneID                string          `json:"pane_id"`
	Rows                  uint16          `json:"rows"`
	Cols                  uint16          `json:"cols"`
	CapturePolicySha256   string          `json:"capture_policy_sha256"`
}

type driverStop struct {
	SchemaVersion         uint32 `json:"schema_version"`
	CaseManifestSha256    string `json:"case_manifest_sha256"`
	DriverOwnershipSha256 string `json:"driver_ownership_sha256"`
	Reason                string `json:"reason"`
}

type driverPrivacy struct {
	SchemaVersion       uint32 `json:"schema_version"`
	CaseManifestSha256  string `json:"case_manifest_sha256"`
	StartedRecordSha256 string `json:"started_record_sha256"`
	Rows                uint16 `json:"rows"`
	Cols                uint16 `json:"cols"`
	CapturePolicySha256 string `json:"capture_policy_sha256"`
}

type driverVersion struct {
	SchemaVersion uint32   `json:"schema_version"`
	Executable    string   `json:"executable"`
	Sha256        string   `json:"sha256"`
	Argv          []string `json:"argv"`
	Stdout        string   `json:"stdout"`
	ExitCode      int32    `json:"exit_code"`
}

func driverFileDigest(path string) (string, error) {
	digest, err := sha256FileHex(path)
	if err != nil {
		return "", ioError(ioErrorText(err))
	}
	return digest, nil
}

func driverExists(path string) (bool, error) {
	exists, err := pathExists(path)
	if err != nil {
		return false, ioError(ioErrorText(err))
	}
	return exists, nil
}

// driverSamePath compares two paths as Rust's Path equality does, by their components.
func driverSamePath(a, b string) bool {
	return filepath.Clean(a) == filepath.Clean(b)
}

// driverPathWithin is Rust's path.starts_with(base): base's components lead path's.
func driverPathWithin(path, base string) bool {
	path, base = filepath.Clean(path), filepath.Clean(base)
	if path == base || base == "/" {
		return strings.HasPrefix(path, base)
	}
	return strings.HasPrefix(path, base+"/")
}

// driverLineCount is Rust's str::lines().count(): a final line terminator ends the last line.
func driverLineCount(text string) int {
	count := strings.Count(text, "\n")
	if text != "" && !strings.HasSuffix(text, "\n") {
		count++
	}
	return count
}

func driverJSONIs(value jsontext.Value, want any) bool {
	return jsonEqual(value, jsonValue(want))
}

// driverOkValue is the JSON value of a result's success, null for its failure.
func driverOkValue(value any, err error) any {
	if err != nil {
		return nil
	}
	return value
}

// driverErrorText is the JSON value of a result's failure message, null for its success.
func driverErrorText(err error) any {
	if err == nil {
		return nil
	}
	return cafleetErrorOf(err).message()
}

func driverEuid() uint32 {
	return uint32(os.Geteuid()) //nolint:gosec // uids are non-negative
}

func driverOwnedPrivateDirectory(info os.FileInfo) bool {
	return info.IsDir() && fileStat(info).Uid == driverEuid() && info.Mode().Perm() == 0o700
}

func driverRead(path string, target any) error {
	data, err := captureReadPrivate(path, 65536)
	if err != nil {
		return appErrorf("%s: %s", path, cafleetErrorOf(err).message())
	}
	if err := decodeStrict(data, target); err != nil {
		return appErrorf("%s: %s", path, err.Error())
	}
	return nil
}

func driverReadValue(path string) (jsontext.Value, error) {
	data, err := captureReadPrivate(path, 65536)
	if err != nil {
		return nil, appErrorf("%s: %s", path, cafleetErrorOf(err).message())
	}
	value, valid := jsonParse(string(data))
	if !valid {
		return nil, appErrorf("%s: invalid JSON", path)
	}
	return value, nil
}

func driverPublish(path string, value any, until time.Time) error {
	encoded, err := encodeJSON(value)
	if err != nil {
		return ioError(cafleetErrorOf(err).message())
	}
	if err := ensure(len(encoded) <= 65536, "private driver record exceeds bound"); err != nil {
		return err
	}
	return capturePublishPrivate(path, encoded, until)
}

func driverTmuxRun(owned *driverCase, parts []string, until time.Time) (string, error) {
	argv := append([]string{"tmux", "-S", owned.Tmux.SocketPath}, parts...)
	output, cause := systemRunner{}.runFor(argv, max(time.Until(until), 0))
	if cause != nil {
		return "", appErrorf("owned tmux command failed: %+v", *cause)
	}
	return output, nil
}

func driverValidateTmux(owned *driverCase, ownership *driverOwnership, until time.Time) error {
	info, err := os.Lstat(owned.Tmux.SocketPath)
	if err != nil {
		return ioError(ioErrorText(err))
	}
	if err := ensure(captureFileIdentityOf(info) == (captureFileIdentity{dev: owned.Tmux.SocketDevice, ino: owned.Tmux.SocketInode}),
		"dedicated tmux socket identity changed"); err != nil {
		return err
	}
	matches, err := ownership.Server.matches()
	if err != nil {
		return err
	}
	if err := ensure(matches, "dedicated tmux server generation changed"); err != nil {
		return err
	}
	actual, err := driverTmuxRun(owned, []string{"display-message", "-p", "-t", owned.Tmux.PaneID, "#{pid}|#{session_name}|#{window_id}|#{pane_id}"}, until)
	if err != nil {
		return err
	}
	return ensure(strings.TrimSpace(actual) == fmt.Sprintf("%d|%s|%s|%s", owned.Tmux.ServerPID, owned.Tmux.Session, owned.Tmux.WindowID, owned.Tmux.PaneID),
		"dedicated controller identity changed")
}

func driverValidateRun(ctx context.Context, conn *dbConn, launched launchOutcome) (runRecord, error) {
	current, err := getRun(ctx, conn, launched.run.runID)
	if err != nil {
		return runRecord{}, err
	}
	if err := launched.started.Identity.validate(current); err != nil {
		return runRecord{}, err
	}
	if err := ensure(current.owner().equal(launched.run.owner()) && current.childPID == some(launched.started.Child.pid) &&
		current.childStartIdentity == some(launched.started.Child.startIdentity), "owned native generation changed"); err != nil {
		return runRecord{}, err
	}
	member, err := getMember(ctx, conn, current.memberID, current.fleetID)
	if err != nil {
		return runRecord{}, err
	}
	record, found := member.get()
	if !found {
		return runRecord{}, appError("owned placement unavailable")
	}
	if err := ensure(record.placement == launched.placement, "owned placement changed"); err != nil {
		return runRecord{}, err
	}
	return current, nil
}

func driverLaunchedPane(launched launchOutcome) string {
	pane, set := launched.placement.muxPaneID.get()
	if !set {
		panic("launched pane identity")
	}
	return pane
}

func driverValidatePane(owned *driverCase, ownership *driverOwnership, launched launchOutcome, until time.Time) error {
	if err := driverValidateTmux(owned, ownership, until); err != nil {
		return err
	}
	pane := driverLaunchedPane(launched)
	actual, err := driverTmuxRun(owned, []string{"display-message", "-p", "-t", pane, "#{pane_pid}|#{session_name}|#{window_id}|#{pane_id}|#{pane_dead}"}, until)
	if err != nil {
		return err
	}
	generation := fmt.Sprintf("%d|%s|%s|%s|", launched.started.Runner.pid, owned.Tmux.Session, owned.Tmux.WindowID, pane)
	actual = strings.TrimSpace(actual)
	if actual == generation+"1" {
		return nil
	}
	live := actual == generation+"0"
	if live {
		if live, err = launched.started.Runner.matches(); err != nil {
			return err
		}
	}
	return ensure(live, "runner pane generation changed")
}

func driverClaimedArm(owned *driverCase) string {
	return strings.TrimSuffix(owned.ArmPath, filepath.Ext(owned.ArmPath)) + ".claimed"
}

func driverArmed(owned *driverCase) (bool, error) {
	armed, err := driverExists(owned.ArmPath)
	if err != nil || armed {
		return armed, err
	}
	return driverExists(driverClaimedArm(owned))
}

func driverValidatePrivacy(owned *driverCase, approved *driverApprovedCase, ownership *driverOwnership) error {
	if err := owned.CapturePolicy.verify(); err != nil {
		return err
	}
	var privacy driverPrivacy
	if err := driverRead(owned.PrivacyConfirmationPath, &privacy); err != nil {
		return err
	}
	bound := privacy.SchemaVersion == 1 && privacy.CaseManifestSha256 == ownership.CaseManifestSha256
	if bound {
		started, err := driverFileDigest(owned.StartedRecordPath)
		if err != nil {
			return err
		}
		bound = privacy.StartedRecordSha256 == started
	}
	return ensure(bound && privacy.Rows == approved.size.rows && privacy.Cols == approved.size.cols && privacy.CapturePolicySha256 == owned.CapturePolicy.Sha256,
		"privacy confirmation binding differs")
}

func driverCaptureRunnerPane(owned *driverCase, pane string, until time.Time) (string, error) {
	const retainedBytes = 48 * 1024
	text, err := driverTmuxRun(owned, []string{"capture-pane", "-p", "-J", "-S", "-", "-t", pane}, until)
	if err != nil {
		return "", err
	}
	start := max(len(text)-retainedBytes, 0)
	for start < len(text) && !utf8.RuneStart(text[start]) {
		start++
	}
	path := filepath.Join(owned.Cwd, "runner-pane-capture.json")
	record := jsonObject{{"schema_version", 1}, {"pane_id", pane}, {"omitted_leading_bytes", start}, {"text", text[start:]}}
	if err := driverPublish(path, record, until); err != nil {
		return "", err
	}
	return path, nil
}

func driverCompleteCapture(owned *driverCase, approved *driverApprovedCase, launched launchOutcome, service jsontext.Value) (jsontext.Value, error) {
	status, integral := jsonInt64(jsonIndex(service, "status"))
	bound := driverJSONIs(jsonIndex(service, "schema_version"), 1) && jsonIsString(jsonIndex(service, "result"), "ok") && integral && status >= 0 && status <= 255
	if bound {
		producer, err := driverFileDigest(owned.ProducerManifestPath)
		if err != nil {
			return nil, err
		}
		bound = jsonIsString(jsonIndex(service, "producer_sha256"), producer)
	}
	if err := ensure(bound && jsonEqual(jsonIndex(service, "process"), jsonValue(launched.started.Runner)),
		"capture service/export did not return a bound actual status"); err != nil {
		return nil, err
	}
	artifactName := owned.CaseID + ".complete.json"
	manifestPath := filepath.Join(owned.OutputDirectory, owned.CaseID+".manifest.json")
	manifest, err := driverReadValue(manifestPath)
	if err != nil {
		return nil, err
	}
	if err := ensure(jsonIsString(jsonIndex(manifest, "capture_id"), owned.CaseID) && jsonIsString(jsonIndex(manifest, "publication_status"), "complete") &&
		jsonIsString(jsonIndex(manifest, "collection_status"), "complete") && jsonIsString(jsonIndex(manifest, "artifact_ref"), artifactName) &&
		jsonIsString(jsonIndex(manifest, "producer_ref"), owned.ProducerManifestPath), "complete capture manifest required"); err != nil {
		return nil, err
	}
	artifact := filepath.Join(owned.OutputDirectory, artifactName)
	data, err := captureReadPrivate(artifact, nativeCaptureLimitsDefault().maxEncodedResultBytes)
	if err != nil {
		return nil, err
	}
	if err := ensure(jsonIsString(jsonIndex(manifest, "artifact_sha256"), sha256Hex(data)) && driverJSONIs(jsonIndex(manifest, "artifact_bytes"), len(data)),
		"capture artifact/manifest digest mismatch"); err != nil {
		return nil, err
	}
	value, valid := jsonParse(string(data))
	if !valid {
		return nil, ioError("capture artifact is not JSON")
	}
	source := jsonIndex(value, "source")
	samples, isArray := jsonArray(jsonIndex(value, "samples"))
	pair := isArray && len(samples) == 2 && !slices.ContainsFunc(samples, func(sample jsontext.Value) bool {
		return !driverJSONIs(jsonIndex(sample, "rows"), approved.size.rows) || !driverJSONIs(jsonIndex(sample, "cols"), approved.size.cols)
	})
	if err := ensure(jsonIsString(jsonIndex(value, "status"), "complete") && jsonIsString(jsonIndex(value, "capture_id"), owned.CaseID) &&
		driverJSONIs(jsonIndex(source, "run_id"), launched.run.runID) && driverJSONIs(jsonIndex(source, "owner_epoch"), launched.run.ownerEpoch) &&
		jsonIsString(jsonIndex(source, "launch_nonce"), launched.run.launchNonce) &&
		jsonEqual(jsonIndex(source, "runner"), jsonValue(launched.started.Runner)) &&
		jsonEqual(jsonIndex(source, "child"), jsonValue(launched.started.Child)) && pair, "actual complete parser pair required"); err != nil {
		return nil, err
	}
	manifestDigest, err := driverFileDigest(manifestPath)
	if err != nil {
		return nil, err
	}
	return jsonValue(jsonObject{
		{"manifest_path", manifestPath},
		{"manifest_sha256", manifestDigest},
		{"artifact_path", artifact},
		{"artifact_sha256", sha256Hex(data)},
		{"service_status", jsonIndex(service, "status")},
	}), nil
}

// captureVerificationLaunchHost is the native launch host with every deadline clipped to the
// case's, whose runner pane re-executes the capture service child in place of the runner.
type captureVerificationLaunchHost struct {
	inner     *nativeLaunchHost
	owned     *driverCase
	caseUntil time.Time
}

func (h *captureVerificationLaunchHost) clock() clock { return h.inner.clock() }

func (h *captureVerificationLaunchHost) currentExecutable() (string, error) {
	return h.inner.currentExecutable()
}

func (h *captureVerificationLaunchHost) splitRunner(paneName string, argv []string, deadline time.Time) (spawnedPane, error) {
	config, err := readNativeCaptureConfig(h.owned.ConfigPath, nativeCaptureLimitsDefault())
	if err != nil {
		return spawnedPane{}, err
	}
	database, found := strings.CutPrefix(h.owned.DatabaseURL, "sqlite:///")
	if !found {
		panic("case database URLs use the sqlite scheme")
	}
	canonical, err := filepath.EvalSymlinks(database)
	if err != nil {
		return spawnedPane{}, ioError(ioErrorText(err))
	}
	if err := ensure(len(argv) == 6 && argv[0] == h.owned.Libtest.Path && argv[1] == "--database-url" && argv[2] == "sqlite:///"+canonical &&
		argv[3] == "__member-run" && argv[4] == "--descriptor" && driverSamePath(argv[5], config.DescriptorPath), "generated launch tuple changed"); err != nil {
		return spawnedPane{}, err
	}
	data, err := captureReadPrivate(config.DescriptorPath, 65536)
	if err != nil {
		return spawnedPane{}, appErrorf("%s: %s", config.DescriptorPath, cafleetErrorOf(err).message())
	}
	descriptor, err := parseLaunchDescriptor(data)
	if err != nil {
		return spawnedPane{}, appErrorf("%s: %s", config.DescriptorPath, cafleetErrorOf(err).message())
	}
	if err := ensure(descriptor.Identity.RunID == config.ExpectedRunID && descriptor.Identity.LaunchNonce == config.ExpectedLaunchNonce &&
		descriptor.Spawn.argv[0] == h.owned.NativeExecutable.Path, "generated descriptor binding changed"); err != nil {
		return spawnedPane{}, err
	}
	if err := driverPublish(h.owned.DescriptorCopyPath, jsontext.Value(descriptor.encode()), h.caseUntil); err != nil {
		return spawnedPane{}, err
	}
	child := testName(TestRuntimeObservation_NativeCapture_Verification_NativeCaptureServiceChild)
	command := []string{
		"/usr/bin/env",
		nativeCaptureEnv + "=" + h.owned.ConfigPath,
		childTestEnv + "=" + child,
		argv[0],
		"-test.run", "^" + child + "$",
		"-test.count=1",
	}
	return h.inner.splitRunner(paneName, command, minTime(deadline, h.caseUntil))
}

func (h *captureVerificationLaunchHost) waitPrepared(descriptor launchDescriptor, deadline time.Time) (prepared, error) {
	return h.inner.waitPrepared(descriptor, minTime(deadline, h.caseUntil))
}

func (h *captureVerificationLaunchHost) start(ready prepared, p placement, deadline time.Time) (started, error) {
	return h.inner.start(ready, p, minTime(deadline, h.caseUntil))
}

func (h *captureVerificationLaunchHost) shutdownOwned(run runRecord, deadlines shutdownDeadlines) (shutdownOutcome, error) {
	return h.inner.shutdownOwned(run, deadlines.clipped(h.caseUntil))
}

func (h *captureVerificationLaunchHost) closePane(pane spawnedPane, deadline time.Time) error {
	return h.inner.closePane(pane, minTime(deadline, h.caseUntil))
}

func (h *captureVerificationLaunchHost) settleHermes(ctx context.Context, conn *dbConn, run runRecord, paneID string, deadline time.Time) error {
	return h.inner.settleHermes(ctx, conn, run, paneID, minTime(deadline, h.caseUntil))
}

func driverPrepare(ctx context.Context, conn *dbConn, pending pendingRegistration, owned *driverCase, until time.Time) (*runArtifacts, spawnSpec, error) {
	approved, err := driverSelectApproved(owned)
	if err != nil {
		return nil, spawnSpec{}, err
	}
	artifacts, err := runArtifactsCreateWithDeadline(ctx, conn, pending.run.runID, until)
	if err != nil {
		return nil, spawnSpec{}, err
	}
	policy, err := approved.backend.resolvePolicy(approved.requestedMode())
	if err != nil {
		return nil, spawnSpec{}, err
	}
	if err := ensure(policy.resolvedMode == approved.permissionMode, "resolved native policy differs from the reviewed case"); err != nil {
		return nil, spawnSpec{}, err
	}
	helper := none[string]()
	if reference, set := owned.Helper.get(); set {
		helper = some(reference.Path)
	}
	prompt := approved.prompt(owned.Cwd, helper)
	argv, err := approved.backend.buildSpawnArgvWithPolicy(prompt, approved.memberName(), owned.Cwd, none[string](), none[string](), &policy)
	if err != nil {
		return nil, spawnSpec{}, err
	}
	argv[0] = owned.NativeExecutable.Path
	catalog := profileCatalogBundled()
	var capture capturePolicyFile
	if err := driverRead(owned.CapturePolicy.Path, &capture); err != nil {
		return nil, spawnSpec{}, err
	}
	if err := ensure(capture.SchemaVersion == 1 && len(capture.Masks) > 0, "reviewed exact-geometry masks required"); err != nil {
		return nil, spawnSpec{}, err
	}
	if err := ensure(!slices.ContainsFunc(capture.Masks, func(mask captureMask) bool { return mask.Rows != approved.size.rows || mask.Cols != approved.size.cols }),
		"case mask geometry differs"); err != nil {
		return nil, spawnSpec{}, err
	}
	spawn := spawnSpec{argv: argv, cwd: owned.Cwd, environment: approved.childEnvironment(), size: approved.size}
	if err := spawn.validate(); err != nil {
		return nil, spawnSpec{}, err
	}
	if _, err := nativeCapturePolicyFromLaunch(capture.Masks, catalog); err != nil {
		return nil, spawnSpec{}, err
	}
	launch := jsonObject{
		{"schema_version", 1},
		{"run_id", pending.run.runID},
		{"launch_nonce", pending.run.launchNonce},
		{"argv", spawn.argv},
		{"cwd", spawn.cwd},
		{"native_executable", owned.NativeExecutable},
		{"version_evidence", owned.VersionEvidence},
	}
	if err := driverPublish(owned.LaunchEvidencePath, launch, until); err != nil {
		return nil, spawnSpec{}, err
	}
	producer := jsonObject{
		{"schema_version", 1},
		{"kind", "verification_libtest"},
		{"source_manifest_ref", owned.SourceManifest.Path},
		{"source_manifest_sha256", owned.SourceManifest.Sha256},
		{"libtest_executable_path", owned.Libtest.Path},
		{"libtest_executable_sha256", owned.Libtest.Sha256},
		{"parser_version", "0.18.1"},
		{"collector_version", "1"},
		{"catalog_origin", "bundled_unknown"},
		{"native_executable", jsonObject{
			{"path", owned.NativeExecutable.Path},
			{"sha256", owned.NativeExecutable.Sha256},
			{"version", owned.NativeVersion},
			{"version_evidence_ref", owned.VersionEvidence.Path},
		}},
		{"launch_evidence_ref", owned.LaunchEvidencePath},
		{"environment_evidence_ref", owned.EnvironmentEvidence.Path},
	}
	if err := driverPublish(owned.ProducerManifestPath, producer, until); err != nil {
		return nil, spawnSpec{}, err
	}
	config := nativeCaptureConfig{
		SchemaVersion:        1,
		DatabaseURL:          owned.DatabaseURL,
		DescriptorPath:       artifacts.paths.descriptor,
		ExpectedRunID:        pending.run.runID,
		ExpectedLaunchNonce:  pending.run.launchNonce,
		ArmPath:              owned.ArmPath,
		OutputDirectory:      owned.OutputDirectory,
		ProducerManifestPath: owned.ProducerManifestPath,
		CapturePolicyPath:    owned.CapturePolicy.Path,
	}
	if err := driverPublish(owned.ConfigPath, config, until); err != nil {
		return nil, spawnSpec{}, err
	}
	return artifacts, spawn, nil
}
func driverPrivateDirectory(path string) error {
	if err := os.Mkdir(path, 0o700); err != nil {
		return appErrorf("%s: %s", path, ioErrorText(err))
	}
	info, err := os.Lstat(path)
	if err != nil {
		return ioError(ioErrorText(err))
	}
	return ensure(driverOwnedPrivateDirectory(info), "fresh private owned directory required")
}

func driverPrivateFile(path string, data []byte) error {
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL|syscall.O_NOFOLLOW, 0o600)
	if err != nil {
		return appErrorf("%s: %s", path, ioErrorText(err))
	}
	if _, err := file.Write(data); err != nil {
		_ = file.Close()
		return ioError(ioErrorText(err))
	}
	if err := file.Sync(); err != nil {
		_ = file.Close()
		return ioError(ioErrorText(err))
	}
	if err := file.Close(); err != nil {
		return ioError(ioErrorText(err))
	}
	return nil
}

func driverCreateOutside(owned *driverCase, approved *driverApprovedCase) error {
	probe, probed := approved.fileProbe.get()
	outside, hasOutside := approved.outsideDirectory().get()
	if !probed || !hasOutside {
		return nil
	}
	if err := ensure(!driverPathWithin(outside, owned.Cwd) && !driverPathWithin(owned.Cwd, outside), "the outside directory lies inside the case cwd"); err != nil {
		return err
	}
	if err := driverPrivateDirectory(outside); err != nil {
		return err
	}
	directory := outside
	if parent := filepath.Dir(probe.probe()); parent != "." {
		for _, component := range strings.Split(parent, "/") {
			directory = filepath.Join(directory, component)
			if err := driverPrivateDirectory(directory); err != nil {
				return err
			}
		}
	}
	if err := driverPrivateFile(filepath.Join(outside, probe.probe()), []byte(driverReadProbeContent)); err != nil {
		return err
	}
	if approved.batched {
		if err := driverPrivateFile(filepath.Join(outside, driverBatchedReadProbe), []byte(driverReadProbeContent)); err != nil {
			return err
		}
	}
	return driverPrivateFile(filepath.Join(outside, "edit-target.txt"), []byte(driverEditTargetContent))
}

func driverOutsideState(approved *driverApprovedCase) (jsontext.Value, error) {
	outside, set := approved.outsideDirectory().get()
	if !set {
		return jsonNull, nil
	}
	state := jsonObject{}
	var visit func(directory string) error
	visit = func(directory string) error {
		entries, err := os.ReadDir(directory)
		if err != nil {
			return ioError(ioErrorText(err))
		}
		for _, entry := range entries {
			path := filepath.Join(directory, entry.Name())
			info, err := os.Lstat(path)
			if err != nil {
				return ioError(ioErrorText(err))
			}
			name, err := filepath.Rel(outside, path)
			if err != nil {
				panic("entries lie below the outside directory")
			}
			if info.IsDir() {
				if err := visit(path); err != nil {
					return err
				}
				continue
			}
			if err := ensure(info.Mode().IsRegular() && info.Size() <= 4096, "bounded regular outside file required"); err != nil {
				return err
			}
			data, err := os.ReadFile(path)
			if err != nil {
				return ioError(ioErrorText(err))
			}
			state = append(state, jsonMember{name, lossyUTF8(data)})
		}
		return nil
	}
	if err := visit(outside); err != nil {
		return nil, err
	}
	return jsonValue(state), nil
}

// driverJSONObject is data's JSON object, a repeated member keeping its first position and last
// value as serde_json's insertion-ordered map does; false when data is JSON but no object.
func driverJSONObject(data []byte) (jsonObject, bool, error) {
	value, valid := jsonParse(string(data))
	if !valid {
		return nil, false, ioError("invalid JSON")
	}
	members, isObject := jsonMembers(value)
	if !isObject {
		return nil, false, nil
	}
	object := jsonObject{}
	for _, member := range members {
		object = object.with(member.key, member.value)
	}
	return object, true, nil
}

func driverMember(object jsonObject, key string) (any, bool) {
	index := slices.IndexFunc(object, func(member jsonMember) bool { return member.key == key })
	if index < 0 {
		return nil, false
	}
	return object[index].value, true
}

type driverSeed struct {
	credentials []byte
	global      jsonObject
	settings    jsonObject
}

func driverValidatedSeed(credentials, global, settings []byte, localSettingsPresent bool) (driverSeed, error) {
	if err := ensure(!localSettingsPresent, "the Claude seed configuration must hold no local settings file"); err != nil {
		return driverSeed{}, err
	}
	settingsObject, isObject, err := driverJSONObject(settings)
	if err != nil {
		return driverSeed{}, err
	}
	theme, themed := driverMember(settingsObject, "theme")
	if !isObject || len(settingsObject) != 1 || !themed || jsonValue(theme).Kind() != '"' {
		return driverSeed{}, appError("the seed settings.json must be exactly a theme-only object")
	}
	globalObject, isObject, err := driverJSONObject(global)
	if err != nil {
		return driverSeed{}, err
	}
	if !isObject {
		return driverSeed{}, appError("seed .claude.json must be an object")
	}
	_, nudged := driverMember(globalObject, "hasSeenAutoDefaultNudge")
	_, envSetup := driverMember(globalObject, "autoModeEnvSetup")
	if err := ensure(!nudged && !envSetup, "the seed .claude.json must hold no auto-mode nudge state"); err != nil {
		return driverSeed{}, err
	}
	return driverSeed{credentials: credentials, global: globalObject, settings: settingsObject}, nil
}

func driverReadSeed() (driverSeed, error) {
	seed := filepath.Join(driverPrivateRoot(), driverClaudeSeed)
	credentials, err := captureReadPrivate(filepath.Join(seed, ".credentials.json"), 65536)
	if err != nil {
		return driverSeed{}, err
	}
	global, err := captureReadPrivate(filepath.Join(seed, ".claude.json"), 4*1024*1024)
	if err != nil {
		return driverSeed{}, err
	}
	settings, err := captureReadPrivate(filepath.Join(seed, "settings.json"), 4096)
	if err != nil {
		return driverSeed{}, err
	}
	local, err := driverExists(filepath.Join(seed, "settings.local.json"))
	if err != nil {
		return driverSeed{}, err
	}
	return driverValidatedSeed(credentials, global, settings, local)
}

// driverExpand is value with {outside} and {cwd} expanded in every string it holds.
func driverExpand(value jsontext.Value, cwd string, outside opt[string]) (any, error) {
	if text, isString := jsonString(value); isString {
		if strings.Contains(text, "{outside}") {
			directory, set := outside.get()
			if !set {
				return nil, appError("an overlay names {outside} without a file probe")
			}
			text = strings.ReplaceAll(text, "{outside}", directory)
		}
		return strings.ReplaceAll(text, "{cwd}", cwd), nil
	}
	if items, isArray := jsonArray(value); isArray {
		expanded := make([]any, 0, len(items))
		for _, item := range items {
			next, err := driverExpand(item, cwd, outside)
			if err != nil {
				return nil, err
			}
			expanded = append(expanded, next)
		}
		return expanded, nil
	}
	if members, isObject := jsonMembers(value); isObject {
		object := jsonObject{}
		for _, member := range members {
			next, err := driverExpand(member.value, cwd, outside)
			if err != nil {
				return nil, err
			}
			object = object.with(member.key, next)
		}
		return object, nil
	}
	return value, nil
}

func driverOverlay(overlay, cwd string, outside opt[string]) (jsonObject, error) {
	object, isObject, err := driverJSONObject([]byte(overlay))
	if err != nil {
		return nil, err
	}
	if !isObject {
		return nil, appError("a configuration overlay is a JSON object")
	}
	for index := range object {
		expanded, err := driverExpand(jsonValue(object[index].value), cwd, outside)
		if err != nil {
			return nil, err
		}
		object[index].value = expanded
	}
	return object, nil
}

func driverSettingsOverlay(overlay, cwd string, outside opt[string]) (jsonObject, error) {
	object, err := driverOverlay(overlay, cwd, outside)
	if err != nil {
		return nil, err
	}
	if err := ensure(!slices.ContainsFunc(object, func(member jsonMember) bool { return member.key != "tui" && member.key != "permissions" }),
		"a settings overlay may hold only tui and permissions"); err != nil {
		return nil, err
	}
	return object, nil
}

// driverPrettyJSON is object as serde_json's to_vec_pretty writes it.
func driverPrettyJSON(object jsonObject) ([]byte, error) {
	encoded, err := json.Marshal(object, jsontext.WithIndent("  "), jsontext.SpaceAfterColon(true))
	if err != nil {
		return nil, ioError(err.Error())
	}
	return encoded, nil
}

type driverConfigFile struct {
	name string
	data []byte
}

func driverClaudeFiles(config driverClaudeConfig, seed driverSeed, cwd string, outside opt[string]) ([]driverConfigFile, error) {
	settingsOverlay, globalOverlay := config.overlays()
	settingsObject, err := driverSettingsOverlay(settingsOverlay, cwd, outside)
	if err != nil {
		return nil, err
	}
	if config.fresh {
		settings, err := driverPrettyJSON(settingsObject)
		if err != nil {
			return nil, err
		}
		return []driverConfigFile{{".credentials.json", seed.credentials}, {"settings.json", settings}}, nil
	}
	global := seed.global
	projectsValue, found := driverMember(global, "projects")
	if !found {
		projectsValue = jsonObject{}
		global = global.with("projects", projectsValue)
	}
	projects, isObject, err := driverJSONObject(jsonValue(projectsValue))
	if err != nil {
		return nil, err
	}
	if !isObject {
		return nil, appError("seed projects must be an object")
	}
	_, named := driverMember(projects, cwd)
	if err := ensure(!named, "the seed already names the case cwd"); err != nil {
		return nil, err
	}
	projects = append(projects, jsonMember{cwd, jsonObject{{"hasTrustDialogAccepted", true}}})
	global = global.with("projects", projects)
	overlay, err := driverOverlay(globalOverlay, cwd, outside)
	if err != nil {
		return nil, err
	}
	for _, member := range overlay {
		global = global.with(member.key, member.value)
	}
	settings := seed.settings
	for _, member := range settingsObject {
		settings = settings.with(member.key, member.value)
	}
	encodedGlobal, err := driverPrettyJSON(global)
	if err != nil {
		return nil, err
	}
	encodedSettings, err := driverPrettyJSON(settings)
	if err != nil {
		return nil, err
	}
	return []driverConfigFile{{".credentials.json", seed.credentials}, {".claude.json", encodedGlobal}, {"settings.json", encodedSettings}}, nil
}

// The credentials file refreshes its token on its own schedule and holds no setting.
func driverConfigSnapshot(directory string) (map[string]jsontext.Value, error) {
	entries, err := os.ReadDir(directory)
	if err != nil {
		return nil, ioError(ioErrorText(err))
	}
	snapshot := map[string]jsontext.Value{}
	for _, entry := range entries {
		name := entry.Name()
		path := filepath.Join(directory, name)
		if name == ".credentials.json" || !strings.HasSuffix(name, ".json") {
			continue
		}
		info, err := os.Lstat(path)
		if err != nil {
			return nil, ioError(ioErrorText(err))
		}
		if !info.Mode().IsRegular() {
			continue
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return nil, ioError(ioErrorText(err))
		}
		value, valid := jsonParse(string(data))
		if !valid {
			value = jsonValue(jsonObject{{"unparsed_sha256", sha256Hex(data)}})
		}
		snapshot[name] = value
	}
	return snapshot, nil
}

// driverKeyPathSet is paths sorted and deduplicated, as a BTreeSet of key paths.
func driverKeyPathSet(paths [][]string) [][]string {
	set := slices.Clone(paths)
	if set == nil {
		set = [][]string{}
	}
	slices.SortFunc(set, slices.Compare[[]string])
	return slices.CompactFunc(set, slices.Equal[[]string])
}

// driverValueOrNull is the member of values named key, null when absent as a key one side lacks.
func driverValueOrNull(values map[string]jsontext.Value, key string) jsontext.Value {
	if value, present := values[key]; present {
		return value
	}
	return jsonNull
}

func driverSortedKeys(left, right map[string]jsontext.Value) []string {
	var keys []string
	for key := range left {
		keys = append(keys, key)
	}
	for key := range right {
		keys = append(keys, key)
	}
	slices.Sort(keys)
	return slices.Compact(keys)
}

func driverChangedKeyPaths(before, after jsontext.Value, path []string, changed *[][]string) {
	if jsonIsObject(before) && jsonIsObject(after) {
		left, right := jsonObjectMap(before), jsonObjectMap(after)
		for _, key := range driverSortedKeys(left, right) {
			driverChangedKeyPaths(driverValueOrNull(left, key), driverValueOrNull(right, key), append(slices.Clone(path), key), changed)
		}
		return
	}
	if !jsonEqual(before, after) {
		*changed = append(*changed, slices.Clone(path))
	}
}

func driverChangedConfiguration(before, after map[string]jsontext.Value, cwd string, outside opt[string]) [][]string {
	var changed [][]string
	for _, file := range driverSortedKeys(before, after) {
		driverChangedKeyPaths(driverValueOrNull(before, file), driverValueOrNull(after, file), []string{file}, &changed)
	}
	for _, path := range changed {
		for index, segment := range path {
			switch {
			case segment == cwd:
				path[index] = "<cwd>"
			case outside == some(segment):
				path[index] = "<outside>"
			}
		}
	}
	return driverKeyPathSet(changed)
}

func driverCompareConfiguration(changed, baseline [][]string, documented *driverDocumentedChange, after map[string]jsontext.Value) error {
	expected := slices.Clone(baseline)
	if documented != nil {
		if len(documented.keyPath) == 0 {
			panic("a documented change names its file")
		}
		value, present := after[documented.keyPath[0]]
		for _, key := range documented.keyPath[1:] {
			if !present {
				break
			}
			value, present = jsonLookup(value, key)
		}
		var holds bool
		switch documented.value {
		case driverChangedToTrue:
			holds = present && value.Kind() == 't'
		case driverChangedToSet:
			holds = present && !jsonIsNull(value)
		default:
			panic("invalid driverChangedTo")
		}
		if err := ensure(holds, "the answer's documented configuration change is absent"); err != nil {
			return err
		}
		expected = append(expected, slices.Clone(documented.keyPath))
	}
	return ensure(slices.EqualFunc(changed, driverKeyPathSet(expected), slices.Equal[[]string]),
		"changed configuration key paths differ from the baseline plus the documented change")
}

func driverDecodeKeyPaths(value jsontext.Value) ([][]string, error) {
	invalid := ioError("changed key paths must be an array of string arrays")
	items, isArray := jsonArray(value)
	if !isArray {
		return nil, invalid
	}
	paths := make([][]string, 0, len(items))
	for _, item := range items {
		segments, isArray := jsonArray(item)
		if !isArray {
			return nil, invalid
		}
		path := make([]string, 0, len(segments))
		for _, segment := range segments {
			text, isString := jsonString(segment)
			if !isString {
				return nil, invalid
			}
			path = append(path, text)
		}
		paths = append(paths, path)
	}
	return driverKeyPathSet(paths), nil
}

func driverBaselineChanges(baseline string) ([][]string, error) {
	check, err := driverReadValue(filepath.Join(driverNeutralRoot, baseline, driverConfigCheck))
	if err != nil {
		return nil, err
	}
	if err := ensure(driverJSONIs(jsonIndex(check, "schema_version"), 1) && jsonIsString(jsonIndex(check, "case_id"), baseline),
		"the baseline configuration check binding differs"); err != nil {
		return nil, err
	}
	return driverDecodeKeyPaths(jsonIndex(check, "changed_key_paths"))
}

func driverPersistentSettingCheck(owned *driverCase, approved *driverApprovedCase, before map[string]jsontext.Value) error {
	config, set := approved.claudeConfigDirectory().get()
	if !set {
		panic("configured cases own a private configuration")
	}
	after, err := driverConfigSnapshot(config)
	if err != nil {
		return err
	}
	changed := driverChangedConfiguration(before, after, owned.Cwd, approved.outsideDirectory())
	settingsFiles := []string{}
	project := filepath.Join(owned.Cwd, ".claude")
	for _, path := range []string{filepath.Join(config, "settings.local.json"), filepath.Join(project, "settings.json"), filepath.Join(project, "settings.local.json")} {
		exists, err := driverExists(path)
		if err != nil {
			return err
		}
		if exists {
			settingsFiles = append(settingsFiles, path)
		}
	}
	var baseline [][]string
	var baselineErr error
	var baselineValue any
	baselineID, hasBaseline := approved.baseline.get()
	if hasBaseline {
		baseline, baselineErr = driverBaselineChanges(baselineID)
		baselineValue = driverOkValue(baseline, baselineErr)
	}
	var documented *driverDocumentedChange
	var documentedValue any
	if nudge, set := approved.nudge.get(); set {
		change := nudge.documentedChange()
		documented, documentedValue = &change, change.keyPath
	}
	record := jsonObject{
		{"schema_version", 1},
		{"case_id", approved.id},
		{"changed_key_paths", changed},
		{"baseline_case_id", approved.baseline},
		{"baseline_changed_key_paths", baselineValue},
		{"baseline_error", driverErrorText(baselineErr)},
		{"documented_change", documentedValue},
		{"settings_files", settingsFiles},
	}
	if err := driverPublish(filepath.Join(owned.Cwd, driverConfigCheck), record, time.Now().Add(5*time.Second)); err != nil {
		return err
	}
	if err := ensure(len(settingsFiles) == 0, "a settings file other than the configuration settings.json appeared"); err != nil {
		return err
	}
	if baselineErr != nil {
		return baselineErr
	}
	if !hasBaseline {
		return nil
	}
	return driverCompareConfiguration(changed, baseline, documented, after)
}

func driverCreatePanelRepository(owned *driverCase, approved *driverApprovedCase) error {
	if !approved.panelProbe {
		return nil
	}
	if err := driverPrivateFile(filepath.Join(owned.Cwd, driverPanelCommitted), []byte("cafleet diff sidebar repository\n")); err != nil {
		return err
	}
	for _, arguments := range [][]string{
		{"init", "--quiet"},
		{"add", driverPanelCommitted},
		{"-c", "user.name=cafleet", "-c", "user.email=cafleet@invalid", "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "sidebar probe base"},
	} {
		command := exec.Command("git", arguments...)
		command.Dir = owned.Cwd
		command.Env = append(os.Environ(), "GIT_CONFIG_GLOBAL=/dev/null", "GIT_CONFIG_NOSYSTEM=1")
		err := command.Run()
		var exit *exec.ExitError
		if err != nil && !errors.As(err, &exit) {
			return ioError(ioErrorText(err))
		}
		if err := ensure(err == nil, "the sidebar repository setup failed"); err != nil {
			return err
		}
	}
	return driverPrivateFile(filepath.Join(owned.Cwd, driverPanelProbe), []byte(driverEditTargetContent))
}

func driverPanelSeparatorColumn(screen *vt.Screen) opt[uint16] {
	rows, cols := screen.Size()
	shared := none[uint16]()
	for row := range rows {
		background := func(col uint16) vt.Color {
			cell := screen.Cell(row, col)
			if cell == nil {
				panic("cells lie on the screen")
			}
			return cell.BgColor()
		}
		separator := none[uint16]()
		for col := uint16(40); col+1 < cols; col++ {
			cell := screen.Cell(row, col)
			if cell != nil && cell.Contents() == "│" && background(col) != background(col-1) && background(col) == background(col+1) {
				separator = some(col)
				break
			}
		}
		column, found := separator.get()
		known, recorded := shared.get()
		switch {
		case !found:
		case !recorded:
			shared = some(column)
		case known != column:
			return none[uint16]()
		}
	}
	return shared
}

func driverConfirmPanelSeparator(owned *driverCase, approved *driverApprovedCase, pane string, until time.Time) error {
	text, err := driverTmuxRun(owned, []string{"capture-pane", "-p", "-e", "-N", "-t", pane}, until)
	if err != nil {
		return err
	}
	parser := vt.NewParser(approved.size.rows, approved.size.cols, nil)
	parser.Process([]byte(strings.ReplaceAll(strings.TrimSuffix(text, "\n"), "\n", "\r\n")))
	column := driverPanelSeparatorColumn(parser.Screen())
	if err := driverPublish(filepath.Join(owned.Cwd, "panel-separator.json"),
		jsonObject{{"schema_version", 1}, {"case_id", approved.id}, {"separator_column", column}}, until); err != nil {
		return err
	}
	return ensure(column.set, "the diff sidebar separator is not visible before the capture")
}

func driverOutputSeq(ctx context.Context, conn *dbConn, launched launchOutcome, until time.Time) (uint64, error) {
	run, err := getRun(ctx, conn, launched.run.runID)
	if err != nil {
		return 0, err
	}
	health, err := probeHealth(run, until, systemClock{})
	if err != nil {
		return 0, err
	}
	return health.OutputSeq, nil
}

func driverEventRow(row sqlRow) (jsonObject, error) {
	eventSeq, err := row.int64(0)
	if err != nil {
		return nil, err
	}
	kind, err := row.text(1)
	if err != nil {
		return nil, err
	}
	observedAt, err := row.text(2)
	if err != nil {
		return nil, err
	}
	interactionID, err := row.optInt64(3)
	if err != nil {
		return nil, err
	}
	classification, err := row.optText(4)
	if err != nil {
		return nil, err
	}
	closeReason, err := row.optText(5)
	if err != nil {
		return nil, err
	}
	closedAt, err := row.optText(6)
	if err != nil {
		return nil, err
	}
	return jsonObject{
		{"event_seq", eventSeq},
		{"kind", kind},
		{"observed_at", observedAt},
		{"interaction_id", interactionID},
		{"classification", classification},
		{"close_reason", closeReason},
		{"closed_at", closedAt},
	}, nil
}

func driverPostStage(ctx context.Context, conn *dbConn, owned *driverCase, approved *driverApprovedCase, launched launchOutcome, armedOutputSeq opt[uint64]) (jsontext.Value, error) {
	until := time.Now().Add(time.Second)
	stopOutputSeq, stopErr := driverOutputSeq(ctx, conn, launched, until)
	events, err := queryRows(conn,
		"SELECT e.event_seq,e.kind,e.observed_at,e.interaction_id,i.classification,i.close_reason,i.closed_at FROM member_runtime_events e LEFT JOIN member_interactions i ON i.interaction_id=e.interaction_id WHERE e.run_id=?1 ORDER BY e.event_seq DESC LIMIT 64",
		[]any{launched.run.runID}, driverEventRow)
	if err != nil {
		return nil, ioError(cafleetErrorOf(err).message())
	}
	slices.Reverse(events)
	markerDigest := none[string]()
	if marker, set := approved.requestedMarker().get(); set {
		path := filepath.Join(owned.Cwd, marker)
		exists, err := driverExists(path)
		if err != nil {
			return nil, err
		}
		if exists {
			digest, err := driverFileDigest(path)
			if err != nil {
				return nil, err
			}
			markerDigest = some(digest)
		}
	} else if err := ensure(approved.fileProbe.set || approved.nudge.set, "answer-stage cases name their probe or nudge"); err != nil {
		return nil, err
	}
	outside, err := driverOutsideState(approved)
	if err != nil {
		return nil, err
	}
	return jsonValue(jsonObject{
		{"decision", approved.decision},
		{"prefix_hex", approved.prefixHex},
		{"stage_hex", approved.stageHex},
		{"output_seq_at_arm", armedOutputSeq},
		{"output_seq_at_stop", driverOkValue(stopOutputSeq, stopErr)},
		{"output_seq_error", driverErrorText(stopErr)},
		{"manual_input_count", "unavailable: runner health exposes no manual-input counter"},
		{"events", events},
		{"requested_marker", approved.requestedMarker()},
		{"requested_marker_sha256", markerDigest},
		{"outside_state", outside},
	}), nil
}

func driverPaneGeometry(owned *driverCase, pane string, until time.Time) (string, error) {
	geometry, err := driverTmuxRun(owned, []string{"display-message", "-p", "-t", pane, "#{pane_width}|#{pane_height}"}, until)
	return strings.TrimSpace(geometry), err
}

func driverWork(ctx context.Context, conn *dbConn, owned *driverCase, ownership *driverOwnership, launched launchOutcome, workUntil time.Time) (jsontext.Value, error) {
	if err := driverValidatePane(owned, ownership, launched, workUntil); err != nil {
		return nil, err
	}
	if _, err := driverValidateRun(ctx, conn, launched); err != nil {
		return nil, err
	}
	pane, set := launched.placement.muxPaneID.get()
	if !set {
		return nil, appError("owned runner pane required")
	}
	approved, err := driverSelectApproved(owned)
	if err != nil {
		return nil, err
	}
	cols, rows := strconv.Itoa(int(approved.size.cols)), strconv.Itoa(int(approved.size.rows))
	for _, command := range [][]string{
		{"resize-window", "-t", owned.Tmux.WindowID, "-x", cols, "-y", "80"},
		{"select-layout", "-t", owned.Tmux.WindowID, "main-horizontal"},
		{"resize-pane", "-t", pane, "-x", cols, "-y", rows},
	} {
		if _, err := driverTmuxRun(owned, command, workUntil); err != nil {
			return nil, err
		}
	}
	geometry, err := driverPaneGeometry(owned, pane, workUntil)
	if err != nil {
		return nil, err
	}
	if err := ensure(geometry == approved.tmuxGeometry(), "actual native geometry differs"); err != nil {
		return nil, err
	}
	ownershipHash, err := driverFileDigest(owned.DriverOwnershipPath)
	if err != nil {
		return nil, err
	}
	record := driverStarted{
		SchemaVersion:         1,
		CaseManifestSha256:    ownership.CaseManifestSha256,
		DriverOwnershipSha256: ownershipHash,
		Identity:              launched.started.Identity,
		Runner:                launched.started.Runner,
		Child:                 launched.started.Child,
		PaneID:                pane,
		Rows:                  approved.size.rows,
		Cols:                  approved.size.cols,
		CapturePolicySha256:   owned.CapturePolicy.Sha256,
	}
	if err := driverPublish(owned.StartedRecordPath, record, workUntil); err != nil {
		return nil, err
	}
	armedValidatedAt := none[time.Time]()
	armedOutputSeq := none[uint64]()
	for {
		stopRequested, err := driverExists(owned.StopPath)
		if err != nil {
			return nil, err
		}
		stopped := !time.Now().Before(workUntil)
		if stopRequested {
			var stop driverStop
			if err := driverRead(owned.StopPath, &stop); err != nil {
				return nil, err
			}
			if err := ensure(stop.SchemaVersion == 1 && stop.Reason == "director_stop" && stop.CaseManifestSha256 == ownership.CaseManifestSha256 &&
				stop.DriverOwnershipSha256 == ownershipHash, "stop binding differs"); err != nil {
				return nil, err
			}
			stopped = true
		}
		if stopped {
			switch approved.mode {
			case driverModeNoAnswer:
				return jsonNull, nil
			case driverModeAnswerStage:
				return driverPostStage(ctx, conn, owned, approved, launched, armedOutputSeq)
			default:
				panic("invalid driverMode")
			}
		}
		live, err := launched.started.Runner.matches()
		if err != nil {
			return nil, err
		}
		if live {
			if live, err = launched.started.Child.matches(); err != nil {
				return nil, err
			}
		}
		if err := ensure(live, "native or runner ended before the requested capture completed"); err != nil {
			return nil, err
		}
		due := true
		if at, set := armedValidatedAt.get(); set {
			due = time.Since(at) >= time.Second
		}
		armed := false
		if due {
			if armed, err = driverArmed(owned); err != nil {
				return nil, err
			}
		}
		if armed {
			armedValidatedAt = some(time.Now())
			if err := driverValidatePrivacy(owned, approved, ownership); err != nil {
				return nil, err
			}
			if err := driverValidateTmux(owned, ownership, workUntil); err != nil {
				return nil, err
			}
			if _, err := driverValidateRun(ctx, conn, launched); err != nil {
				return nil, err
			}
			geometry, err := driverPaneGeometry(owned, pane, workUntil)
			if err != nil {
				return nil, err
			}
			if err := ensure(geometry == approved.tmuxGeometry(), "armed geometry changed"); err != nil {
				return nil, err
			}
			if !armedOutputSeq.set {
				if approved.panelProbe {
					if err := driverConfirmPanelSeparator(owned, approved, pane, workUntil); err != nil {
						return nil, err
					}
				}
				seq, err := driverOutputSeq(ctx, conn, launched, workUntil)
				if err != nil {
					return nil, err
				}
				armedOutputSeq = some(seq)
			}
		}
		for _, marker := range approved.forbiddenMarkers() {
			executed, err := driverExists(filepath.Join(owned.Cwd, marker))
			if err != nil {
				return nil, err
			}
			if err := ensure(!executed, "a probe outside the reviewed decision executed"); err != nil {
				return nil, err
			}
		}
		time.Sleep(min(20*time.Millisecond, max(time.Until(workUntil), 0)))
	}
}
func runNativeCapturePrepareStart(configPath string) error {
	parent, set := pathParent(configPath).get()
	if !set {
		return appError("case parent required")
	}
	caseBytes, err := captureReadPrivate(filepath.Join(parent, "case.json"), 65536)
	if err != nil {
		return err
	}
	var owned driverCase
	if err := decodeStrict(caseBytes, &owned); err != nil {
		return ioError(err.Error())
	}
	approved, err := driverSelectApproved(&owned)
	if err != nil {
		return err
	}
	if err := ensure(owned.SchemaVersion == 1 && owned.CleanupReserveMS == 4000 && driverSamePath(owned.ConfigPath, configPath) && len(owned.CaseNonce) == 64,
		"private case identity differs"); err != nil {
		return err
	}
	neutral, private := driverNeutralRoot, driverPrivateRoot()
	var helperBound bool
	if helper, set := owned.Helper.get(); approved.fileProbe.set {
		helperBound = !set
	} else {
		helperBound = set && driverSamePath(helper.Path, filepath.Join(owned.Cwd, "permission-probe"))
	}
	if err := ensure(driverSamePath(owned.Cwd, filepath.Join(neutral, approved.id)) &&
		owned.DatabaseURL == "sqlite:///"+filepath.Join(private, approved.database) &&
		driverSamePath(owned.Tmux.SocketPath, filepath.Join(private, approved.socket)) &&
		driverSamePath(owned.OutputDirectory, filepath.Join(neutral, approved.id+"-capture")) && helperBound,
		"case resources differ from approved isolated paths"); err != nil {
		return err
	}
	for _, path := range []string{owned.ConfigPath, owned.ArmPath, owned.ProducerManifestPath, owned.LaunchEvidencePath, owned.DriverOwnershipPath,
		owned.StartedRecordPath, owned.StopPath, owned.PrivacyConfirmationPath, owned.ResultPath, owned.DescriptorCopyPath} {
		if err := ensure(driverSamePath(filepath.Dir(path), owned.Cwd), "driver output outside owned case directory"); err != nil {
			return err
		}
	}
	_, known := codingAgentCodingAgent(owned.DirectorBackend)
	if err := ensure(known, "known authorizing Director backend required"); err != nil {
		return err
	}
	for _, directory := range []string{owned.Cwd, owned.OutputDirectory} {
		info, err := os.Lstat(directory)
		if err != nil {
			return ioError(ioErrorText(err))
		}
		if err := ensure(driverOwnedPrivateDirectory(info), "private owned case directory required"); err != nil {
			return err
		}
	}
	database, err := databasePath(owned.DatabaseURL)
	if err != nil {
		return err
	}
	if _, err := privateMetadata(database, driverEuid(), 0o600, false); err != nil {
		return err
	}
	sample, err := systemClock{}.monotonicSample()
	if err != nil {
		return err
	}
	if owned.ExpiresAtMonoNS < sample.monotonicNs {
		return deadlineExceededError()
	}
	remaining := owned.ExpiresAtMonoNS - sample.monotonicNs
	if remaining <= 4_000_000_000 || remaining > 120_000_000_000+owned.AnswerWindowMS*1_000_000 {
		return deadlineExceededError()
	}
	caseUntil := sample.before.Add(time.Duration(remaining)) //nolint:gosec // remaining is at most 120 s plus the reviewed answer window
	workUntil := caseUntil.Add(-4 * time.Second)
	references := []driverReference{owned.NativeExecutable, owned.Libtest, owned.SourceManifest, owned.VersionEvidence, owned.EnvironmentEvidence, owned.CapturePolicy}
	if helper, set := owned.Helper.get(); set {
		references = append(references, helper)
	}
	for _, reference := range references {
		if err := reference.verify(); err != nil {
			return err
		}
	}
	var version driverVersion
	if err := driverRead(owned.VersionEvidence.Path, &version); err != nil {
		return err
	}
	token, tokenErr := versionToken(version.Stdout)
	if err := ensure(version.SchemaVersion == 1 && version.ExitCode == 0 && driverSamePath(version.Executable, owned.NativeExecutable.Path) &&
		version.Sha256 == owned.NativeExecutable.Sha256 && slices.Equal(version.Argv, []string{owned.NativeExecutable.Path, "--version"}) &&
		driverLineCount(version.Stdout) == 1 && tokenErr == nil && token == owned.NativeVersion, "same-file actual version evidence differs"); err != nil {
		return err
	}
	executable, err := os.Executable()
	if err != nil {
		return ioError(ioErrorText(err))
	}
	if err := ensure(driverSamePath(executable, owned.Libtest.Path), "selected libtest differs"); err != nil {
		return err
	}
	fresh := true
	for _, path := range []string{filepath.Join(owned.Cwd, "allow.executed"), filepath.Join(owned.Cwd, "deny.executed"), owned.ArmPath} {
		exists, err := driverExists(path)
		if err != nil {
			return err
		}
		if exists {
			fresh = false
			break
		}
	}
	if err := ensure(fresh, "fresh case without probe markers or arm required"); err != nil {
		return err
	}
	if err := driverCreateOutside(&owned, approved); err != nil {
		return err
	}
	if err := driverCreatePanelRepository(&owned, approved); err != nil {
		return err
	}
	config, hasDirectory := approved.claudeConfigDirectory().get()
	claudeConfig, configured := approved.claudeConfig.get()
	if !hasDirectory || !configured {
		return driverLaunchCase(&owned, approved, caseBytes, caseUntil, workUntil)
	}
	seed, err := driverReadSeed()
	if err != nil {
		return err
	}
	files, err := driverClaudeFiles(claudeConfig, seed, owned.Cwd, approved.outsideDirectory())
	if err != nil {
		return err
	}
	if err := driverPrivateDirectory(config); err != nil {
		return err
	}
	outcome := func() error {
		for _, file := range files {
			if err := driverPrivateFile(filepath.Join(config, file.name), file.data); err != nil {
				return err
			}
		}
		before, err := driverConfigSnapshot(config)
		if err != nil {
			return err
		}
		launched := driverLaunchCase(&owned, approved, caseBytes, caseUntil, workUntil)
		return mergeCleanup(launched, driverPersistentSettingCheck(&owned, approved, before))
	}()
	var teardown error
	if err := os.RemoveAll(config); err != nil {
		teardown = ioError(ioErrorText(err))
	}
	return mergeCleanup(outcome, teardown)
}

// driverAttempt is an operation's outcome or the payload it panicked with, as Rust's
// catch_unwind result.
type driverAttempt[T any] struct {
	value    T
	err      error
	panicked bool
	payload  any
}

func driverCatch[T any](operation func() (T, error)) (attempt driverAttempt[T]) {
	defer func() {
		if payload := recover(); payload != nil {
			attempt = driverAttempt[T]{panicked: true, payload: payload}
		}
	}()
	value, err := operation()
	return driverAttempt[T]{value: value, err: err}
}

func driverLaunchCase(owned *driverCase, approved *driverApprovedCase, caseBytes []byte, caseUntil, workUntil time.Time) error {
	driverProcess, err := processIdentityCurrent()
	if err != nil {
		return err
	}
	server, present, err := processIdentityRead(owned.Tmux.ServerPID)
	if err != nil {
		return err
	}
	if !present {
		return appError("dedicated server absent")
	}
	ownership := driverOwnership{
		SchemaVersion:      1,
		CaseManifestSha256: sha256Hex(caseBytes),
		CaseNonce:          owned.CaseNonce,
		Driver:             driverProcess,
		Server:             server,
		Tmux:               owned.Tmux,
		ExpiresAtMonoNS:    owned.ExpiresAtMonoNS,
	}
	if err := driverValidateTmux(owned, &ownership, workUntil); err != nil {
		return err
	}
	if err := driverPublish(owned.DriverOwnershipPath, ownership, workUntil); err != nil {
		return err
	}
	muxServer := owned.Tmux.SocketPath
	mux, muxErr := multiplexerForServer("tmux", muxServer, systemRunner{})
	if muxErr != nil {
		return ioError(muxErr.Error())
	}
	muxContext := multiplexerContext{session: owned.Tmux.Session, windowID: owned.Tmux.WindowID, paneID: owned.Tmux.PaneID}
	windowName, muxErr := mux.windowName(muxContext.windowID)
	if muxErr != nil {
		return ioError(muxErr.Error())
	}
	target := columnTarget{windowID: muxContext.windowID, side: columnSideRight, columnPanes: []string{}, otherPanes: []string{muxContext.paneID}}
	cwd, err := os.Getwd()
	if err != nil {
		return ioError(ioErrorText(err))
	}
	host := &captureVerificationLaunchHost{inner: newNativeLaunchHost(mux, target, cwd, systemClock{}), owned: owned, caseUntil: caseUntil}
	conn, err := connect(owned.DatabaseURL)
	if err != nil {
		return mergeCleanup(err, host.inner.close())
	}
	defer func() { _ = conn.close() }()
	_, err = withDatabaseDeadline(context.Background(), conn, caseUntil, func(ctx context.Context, conn *dbConn) (struct{}, error) {
		if err := initialize(conn, owned.DatabaseURL); err != nil {
			return struct{}{}, err
		}
		fleet, err := createFleet(ctx, conn, "native-"+approved.id, newPlacementDirector("tmux", muxContext.session, owned.DirectorBackend, muxServer,
			placementPane{muxWindowID: muxContext.windowID, muxWindowName: windowName, muxPaneID: muxContext.paneID}))
		if err != nil {
			return struct{}{}, err
		}
		policy, err := approved.backend.resolvePolicy(approved.requestedMode())
		if err != nil {
			return struct{}{}, err
		}
		identity, err := databaseIdentity(ctx, conn)
		if err != nil {
			return struct{}{}, err
		}
		settings := runSettings{
			databaseIdentityJSON: identity,
			creator:              ownership.Driver,
			launchNonce:          owned.CaseNonce,
			requestedMode:        policy.requestedMode,
			resolvedMode:         policy.resolvedMode,
			modeSource:           policy.modeSource,
			permissionArgvJSON:   jsonValue(policy.permissionArgv),
			nativeVersion:        some(owned.NativeVersion),
			coverage:             profileCatalogBundled().detectionCoverage(approved.backend),
		}
		member := pendingMemberSpec{
			fleetID:     fleet.fleetID,
			name:        approved.memberName(),
			description: fmt.Sprintf("private %s capture", owned.Mode),
			placement: newPlacement{
				backend:       "tmux",
				muxSession:    muxContext.session,
				windowOrdinal: 0,
				columnSide:    columnSideRight,
				codingAgent:   approved.backend.name(),
				muxServer:     muxServer,
				pane:          none[placementPane](),
			},
			windowName:                windowName,
			advancesWindowOrdinalFrom: none[int64](),
		}
		launchUntil := minTime(workUntil, time.Now().Add(10*time.Second))
		launched, err := createRegistered(ctx, conn, member, settings,
			func(ctx context.Context, conn *dbConn, pending pendingRegistration) (*runArtifacts, spawnSpec, error) {
				return driverPrepare(ctx, conn, pending, owned, launchUntil)
			}, host, launchUntil)
		if err != nil {
			return struct{}{}, err
		}
		work := driverCatch(func() (jsontext.Value, error) {
			return driverWork(ctx, conn, owned, &ownership, launched, workUntil)
		})
		workError := none[string]()
		postStage := jsonNull
		switch {
		case work.panicked:
			workError = some("owned post-start work unwound")
		case work.err != nil:
			message := []rune(cafleetErrorOf(work.err).message())
			workError = some(string(message[:min(len(message), 2048)]))
		default:
			postStage = work.value
		}
		cleanup := driverCatch(func() (struct{}, error) {
			return struct{}{}, driverLaunchCleanup(ctx, conn, host, owned, approved, &ownership, launched, caseUntil, workError, postStage)
		})
		switch {
		case work.panicked:
			if cleanup.panicked || cleanup.err != nil {
				fmt.Fprintln(os.Stderr, "native driver cleanup also failed")
			}
			panic(work.payload)
		case work.err != nil && cleanup.panicked:
			return struct{}{}, cafleetErrorOf(work.err).withCleanup("owned cleanup panicked")
		case work.err != nil:
			return struct{}{}, mergeCleanup(work.err, cleanup.err)
		case cleanup.panicked:
			panic(cleanup.payload)
		default:
			return struct{}{}, cleanup.err
		}
	})
	return mergeCleanup(err, host.inner.close())
}

// driverObserveOutcome waits for the runner to exit and binds the service outcome, the complete
// capture and the pane's exit status to the launched run.
func driverObserveOutcome(owned *driverCase, approved *driverApprovedCase, ownership *driverOwnership, launched launchOutcome, pane string, deadlines shutdownDeadlines) (jsontext.Value, error) {
	for {
		live, err := launched.started.Runner.matches()
		if err != nil {
			return nil, err
		}
		if !live || !time.Now().Before(deadlines.storageDeadline()) {
			break
		}
		time.Sleep(10 * time.Millisecond)
	}
	// tmux reports the pane live for a moment after the runner exits.
	for {
		err := driverValidatePane(owned, ownership, launched, deadlines.storageDeadline())
		if err == nil {
			break
		}
		if !time.Now().Before(deadlines.storageDeadline()) {
			return nil, err
		}
		time.Sleep(10 * time.Millisecond)
	}
	service, err := driverReadValue(strings.TrimSuffix(owned.ConfigPath, filepath.Ext(owned.ConfigPath)) + ".service-outcome.json")
	if err != nil {
		return nil, err
	}
	configDigest, err := driverFileDigest(owned.ConfigPath)
	if err != nil {
		return nil, err
	}
	if err := ensure(jsonIsString(jsonIndex(service, "config_sha256"), configDigest) && driverJSONIs(jsonIndex(service, "run_id"), launched.run.runID) &&
		jsonIsString(jsonIndex(service, "launch_nonce"), launched.run.launchNonce), "service outcome binding differs"); err != nil {
		return nil, err
	}
	armed, err := driverArmed(owned)
	if err != nil {
		return nil, err
	}
	if err := ensure(armed, "not_armed: the capture was never armed"); err != nil {
		return nil, err
	}
	if err := driverValidatePrivacy(owned, approved, ownership); err != nil {
		return nil, err
	}
	capture, err := driverCompleteCapture(owned, approved, launched, service)
	if err != nil {
		return nil, err
	}
	paneStatus, err := driverTmuxRun(owned, []string{"display-message", "-p", "-t", pane, "#{pane_dead}|#{pane_dead_status}"}, deadlines.storageDeadline())
	if err != nil {
		return nil, err
	}
	status, integral := jsonInt64(jsonIndex(service, "status"))
	if !integral {
		panic("validated status")
	}
	if err := ensure(strings.TrimSpace(paneStatus) == fmt.Sprintf("1|%d", status), "actual pane exit differs from service status"); err != nil {
		return nil, err
	}
	return jsonValue(jsonObject{{"service", service}, {"pane_status", paneStatus}, {"capture", capture}}), nil
}

// driverLaunchCleanup shuts the launched run down, records the case result and fails unless the
// cleanup completed with every owned process gone and no probe outside the reviewed decision run.
func driverLaunchCleanup(ctx context.Context, conn *dbConn, host *captureVerificationLaunchHost, owned *driverCase, approved *driverApprovedCase,
	ownership *driverOwnership, launched launchOutcome, caseUntil time.Time, workError opt[string], postStage jsontext.Value) error {
	deadlines := shutdownDeadlinesStartingAt(time.Now(), shutdownUnit).clipped(caseUntil)
	var shutdownResult shutdownOutcome
	current, shutdownErr := driverValidateRun(ctx, conn, launched)
	if shutdownErr == nil {
		shutdownResult, shutdownErr = host.shutdownOwned(current, deadlines)
	}
	pane, set := launched.placement.muxPaneID.get()
	if !set {
		panic("launched pane")
	}
	observed, observedErr := driverObserveOutcome(owned, approved, ownership, launched, pane, deadlines)
	var paneCapture string
	paneCaptureErr := driverValidatePane(owned, ownership, launched, deadlines.storageDeadline())
	if paneCaptureErr == nil {
		paneCapture, paneCaptureErr = driverCaptureRunnerPane(owned, pane, deadlines.storageDeadline())
	}
	closeErr := driverValidatePane(owned, ownership, launched, deadlines.storageDeadline())
	if closeErr == nil {
		window, set := launched.placement.muxWindowID.get()
		if !set {
			panic("a launched placement records its window")
		}
		closeErr = host.closePane(spawnedPane{paneID: pane, windowID: window}, deadlines.storageDeadline())
	}
	runnerLive, runnerErr := launched.started.Runner.matches()
	nativeLive, nativeErr := launched.started.Child.matches()
	ownershipDigest, err := driverFileDigest(owned.DriverOwnershipPath)
	if err != nil {
		return err
	}
	outside, err := driverOutsideState(approved)
	if err != nil {
		return err
	}
	markerPresent := none[bool]()
	if marker, set := approved.requestedMarker().get(); set {
		exists, err := driverExists(filepath.Join(owned.Cwd, marker))
		if err != nil {
			return err
		}
		markerPresent = some(exists)
	}
	allowPresent, err := driverExists(filepath.Join(owned.Cwd, "allow.executed"))
	if err != nil {
		return err
	}
	denyPresent, err := driverExists(filepath.Join(owned.Cwd, "deny.executed"))
	if err != nil {
		return err
	}
	summary := jsonObject{
		{"schema_version", 1},
		{"case_manifest_sha256", ownership.CaseManifestSha256},
		{"work_error", workError},
		{"driver_ownership_sha256", ownershipDigest},
		{"shutdown", driverOkValue(shutdownResult, shutdownErr)},
		{"shutdown_error", driverErrorText(shutdownErr)},
		{"observation", driverOkValue(observed, observedErr)},
		{"observation_error", driverErrorText(observedErr)},
		{"pane_capture", driverOkValue(paneCapture, paneCaptureErr)},
		{"pane_capture_error", driverErrorText(paneCaptureErr)},
		{"pane_closed", closeErr == nil},
		{"close_error", driverErrorText(closeErr)},
		{"runner_absent", driverOkValue(!runnerLive, runnerErr)},
		{"native_absent", driverOkValue(!nativeLive, nativeErr)},
		{"runner_identity_error", driverErrorText(runnerErr)},
		{"native_identity_error", driverErrorText(nativeErr)},
		{"case_id", approved.id},
		{"post_stage", postStage},
		{"outside_state", outside},
		{"requested_marker", approved.requestedMarker()},
		{"requested_marker_present", markerPresent},
		{"allow_marker_absent", !allowPresent},
		{"deny_marker_absent", !denyPresent},
	}
	if err := driverPublish(owned.ResultPath, summary, caseUntil); err != nil {
		return err
	}
	if shutdownErr != nil {
		return shutdownErr
	}
	if observedErr != nil {
		return observedErr
	}
	if closeErr != nil {
		return closeErr
	}
	forbiddenAbsent := true
	for _, marker := range approved.forbiddenMarkers() {
		exists, err := driverExists(filepath.Join(owned.Cwd, marker))
		if err != nil {
			return err
		}
		forbiddenAbsent = forbiddenAbsent && !exists
	}
	clean := shutdownResult.cleanupState == cleanupStateComplete
	if clean {
		if runnerErr != nil {
			return runnerErr
		}
		clean = !runnerLive
	}
	if clean {
		if nativeErr != nil {
			return nativeErr
		}
		clean = !nativeLive
	}
	return ensure(clean && forbiddenAbsent, "native case cleanup incomplete or a probe outside the reviewed decision executed")
}

func TestRuntimeObservation_NativeCapture_Verification_NativeCapturePrepareStartDriver(t *testing.T) {
	t.Parallel()
	config := os.Getenv(nativeCaptureEnv)
	if config == "" {
		t.Skip("Director-owned native preparation requires an explicit private case")
	}
	if !filepath.IsAbs(config) {
		t.Fatal("absolute capture configuration required")
	}
	if err := runNativeCapturePrepareStart(config); err != nil {
		t.Fatalf("actual native prepare/start/cleanup outcome: %s", cafleetErrorOf(err).message())
	}
}
