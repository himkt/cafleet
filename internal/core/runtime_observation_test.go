package core

import (
	"context"
	"encoding/json/jsontext"
	"fmt"
	"math"
	"os"
	"os/exec"
	"regexp"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"sync/atomic"
	"syscall"
	"testing"
	"time"
	"unicode/utf8"
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

func TestRuntimeObservation_ContinuationEpisodes_RuntimeDetectorParserBridgeVisitsATransientCandidateBeforeALaterClear(t *testing.T) {
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

func TestRuntimeObservation_ContinuationEpisodes_RuntimeEpisodeTimelinesOpenHoldAndCloseOnTheirBoundaries(t *testing.T) {
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

func TestRuntimeObservation_ContinuationEpisodes_RuntimeEpisodeClassAndStableChoiceChangesSupersedeAfterStability(t *testing.T) {
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

func TestRuntimeObservation_ContinuationEpisodes_RuntimeEpisodeEquivalentRedrawSchedulesNoRefreshAndNoOpening(t *testing.T) {
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

func TestRuntimeObservation_ContinuationEpisodes_RuntimeEpisodeMetadataRemovesEachScreenFieldAndLeavesNoRedrawRefresh(t *testing.T) {
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

func TestRuntimeObservation_ContinuationEpisodes_RuntimeEpisodeMetadataDigestContainsNoHashOfHiddenSemantics(t *testing.T) {
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

func TestRuntimeObservation_ContinuationEpisodes_RuntimeEpisodeMaterializeNumbersTheDraftAndRefusesANonPositiveSequence(t *testing.T) {
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

func TestRuntimeObservation_BundledClaude2_1_312Tracker_TrackerReplacesThePendingNoticeWithDialogAAfterTheDraftIsCleared(t *testing.T) {
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

func TestRuntimeObservation_BundledClaude2_1_312Tracker_TrackerClosesThePendingNoticeWithoutDialogAAfterTheDraftIsSent(t *testing.T) {
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

func TestRuntimeObservation_BundledClaude2_1_312Tracker_TrackerClosesDialogAOnceItsDeclineReturnsTheComposer(t *testing.T) {
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

func TestRuntimeObservation_BundledClaude2_1_312Tracker_TrackerSupersedesTheFirstBatchedReadWithTheSecondAfterItsAllow(t *testing.T) {
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

func TestRuntimeObservation_ObservationFailures_RunnerQuietDetectorFailureReturnsTypedHeldStatusAndPreservesTheEpisode(t *testing.T) {
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

func TestRuntimeObservation_ObservationFailures_RunnerOrderedAdapterPreservesDetectorAttributionAndBoundedMetadata(t *testing.T) {
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

func TestRuntimeObservation_ObservationFailures_RunnerClockAndOutputSequenceInvariantsPropagateOutsideRecoverableFailures(t *testing.T) {
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

func TestRuntimeObservation_ObservationFailures_HealthReportsNoWriterDiagnosticOrGapStartWhileHealthy(t *testing.T) {
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

func TestRuntimeObservation_ObservationFailures_HealthReportsTheSanitizedWriterDiagnosticAndThePendingGapStart(t *testing.T) {
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

func TestRuntimeObservation_ObservationFailures_HealthReportsATerminalFencedGapWithoutAGapStart(t *testing.T) {
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

func TestRuntimeObservation_ObservationFailures_RunnerFailureAfterTerminalEnqueueNeverAppendsALivePostTerminalGap(t *testing.T) {
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

func TestRuntimeObservation_ControllerQuietPollOpensOneEpisodeWithTheUnchangedParserSequence(t *testing.T) {
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

func TestRuntimeObservation_ControllerActualAdapterObservesTransientFormBeforeClearWithoutOpening(t *testing.T) {
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

func TestRuntimeObservation_ControllerTypedFailuresKeepMetadataOnlyDiagnosticsAndHoldInput(t *testing.T) {
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

func TestRuntimeObservation_ControllerStartedValidatesCurrentIdentityAndIsSingleUse(t *testing.T) {
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

func TestRuntimeObservation_ControllerExitClosesTheOpenEpisodeBeforeRecordingTheRealNativeSignal(t *testing.T) {
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

func TestRuntimeObservation_ControllerRetainsBothSupersededOriginalsWhenTheFirstSubmitHitsPressure(t *testing.T) {
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

func TestRuntimeObservation_ControllerGapRecoveryReobservesTheUnchangedScreenAfterPublishingADelivery(t *testing.T) {
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

func TestRuntimeObservation_PublishDeliveryResultQueuesBlockedRecordsPerBlockerAndDropsAQueuedRepeat(t *testing.T) {
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

func TestRuntimeObservation_ControllerReplacesAnEvidenceInvalidRefusalWithItsRejectionAndOpensNoGap(t *testing.T) {
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
