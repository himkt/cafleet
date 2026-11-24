package core

import (
	"fmt"
	"regexp"
	"slices"
	"strings"
	"testing"
	"time"
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
	checkDeadline
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
	return timelineCheck{kind: checkDeadline, value: millis}
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
				case checkDeadline:
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
