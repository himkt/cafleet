package core

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"fmt"
	"math"
	"slices"
	"sync"
	"time"
)

type observationMoment struct {
	monotonic  time.Time
	observedAt string
}

func observationMomentSample(c clock) observationMoment {
	return observationMoment{monotonic: c.now(), observedAt: wallStamp(c)}
}

// observationPhase names the runner step a test hook interrupts; production has no hooks.
type observationPhase uint8

const (
	observationPhaseChildOutput observationPhase = iota + 1
	observationPhaseQuietTimer
	observationPhaseResize
	observationPhaseShutdownDrain
)

type observationOperation uint8

const (
	observationOperationParser observationOperation = iota + 1
	observationOperationDetector
)

// observationHooks lets a test fail an observation step; production observers have none.
type observationHooks interface {
	before(site observationPhase, operation observationOperation) error
	orderShutdownParsingAfterNativeExit(deadline time.Time) bool
}

type observationStatus struct {
	visibility         visibility
	eligibility        inputEligibility
	nextDeadline       opt[time.Time]
	retainedTransition bool
	retainedDraft      bool
	gapCount           uint64
	diagnostic         opt[string]
}

type observationFailure uint8

const (
	observationFailureParser observationFailure = iota + 1
	observationFailureDetector
	observationFailureSink
)

func (v observationFailure) asStr() string {
	switch v {
	case observationFailureParser:
		return "parser_unavailable"
	case observationFailureDetector:
		return "detector_unavailable"
	case observationFailureSink:
		return "sink_unavailable"
	default:
		panic("invalid observationFailure")
	}
}

func observationFailureTryFrom(value string) (observationFailure, error) {
	switch value {
	case "parser_unavailable":
		return observationFailureParser, nil
	case "detector_unavailable":
		return observationFailureDetector, nil
	case "sink_unavailable":
		return observationFailureSink, nil
	default:
		return 0, invalidStoredValueError("observation.diagnostic", value)
	}
}

func (v observationFailure) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *observationFailure) UnmarshalText(text []byte) error {
	parsed, err := observationFailureTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

type visibility uint8

const (
	visibilityVisible visibility = iota + 1
	visibilityObscured
	visibilityNotVisible
	visibilityUnknown
)

func (v visibility) asStr() string {
	switch v {
	case visibilityVisible:
		return "visible"
	case visibilityObscured:
		return "obscured"
	case visibilityNotVisible:
		return "not_visible"
	case visibilityUnknown:
		return "unknown"
	default:
		panic("invalid visibility")
	}
}

func visibilityTryFrom(value string) (visibility, error) {
	switch value {
	case "visible":
		return visibilityVisible, nil
	case "obscured":
		return visibilityObscured, nil
	case "not_visible":
		return visibilityNotVisible, nil
	case "unknown":
		return visibilityUnknown, nil
	default:
		return 0, invalidStoredValueError("observation.visibility", value)
	}
}

func (v visibility) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *visibility) UnmarshalText(text []byte) error {
	parsed, err := visibilityTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

type gateReason uint8

const (
	gateReasonCandidate gateReason = iota + 1
	gateReasonInteraction
	gateReasonUnknown
	gateReasonObserverUnavailable
	gateReasonGap
)

func (r gateReason) asStr() string {
	switch r {
	case gateReasonCandidate:
		return "candidate"
	case gateReasonInteraction:
		return "interaction"
	case gateReasonUnknown:
		return "unknown"
	case gateReasonObserverUnavailable:
		return "observer_unavailable"
	case gateReasonGap:
		return "gap"
	default:
		panic("invalid gateReason")
	}
}

var gateReasonVariants = []gateReason{
	gateReasonCandidate, gateReasonInteraction, gateReasonUnknown, gateReasonObserverUnavailable, gateReasonGap,
}

func (r gateReason) MarshalText() ([]byte, error) { return []byte(r.asStr()), nil }

// UnmarshalText decodes the serde snake_case variant name.
func (r *gateReason) UnmarshalText(text []byte) error {
	for _, variant := range gateReasonVariants {
		if variant.asStr() == string(text) {
			*r = variant
			return nil
		}
	}
	return appErrorf("unknown variant `%s`, expected one of `candidate`, `interaction`, `unknown`, `observer_unavailable`, `gap`", text)
}

// inputEligibility is whether ordinary input may start: a proven empty composer, or a hold with
// its reason. Its variants are comparable, so == compares eligibilities as Rust's PartialEq.
//
//sumtype:decl
type inputEligibility interface {
	isInputEligibility()
	rendering() string
}

type inputEligibilityComposerEmpty struct{}

type inputEligibilityHold struct {
	reason gateReason
}

func (inputEligibilityComposerEmpty) isInputEligibility() {}
func (inputEligibilityHold) isInputEligibility()          {}

func (inputEligibilityComposerEmpty) rendering() string { return "composer_empty" }

func (h inputEligibilityHold) rendering() string { return "hold:" + h.reason.asStr() }

// inputEligibilityJSON is the serde adjacently tagged encoding: {"state":...,"reason":...}.
func inputEligibilityJSON(eligibility inputEligibility) jsonObject {
	switch eligibility := eligibility.(type) {
	case inputEligibilityComposerEmpty:
		return jsonObject{{"state", "composer_empty"}}
	case inputEligibilityHold:
		return jsonObject{{"state", "hold"}, {"reason", eligibility.reason.asStr()}}
	default:
		panic("invalid inputEligibility")
	}
}

// inputEligibilityDecode reads the serde adjacently tagged encoding, which denies unknown and
// duplicate members; a unit variant's content may be absent or null.
func inputEligibilityDecode(value jsontext.Value) (inputEligibility, error) {
	members, isObject := jsonMembers(value)
	if !isObject {
		return nil, appError("invalid type: expected adjacently tagged enum InputEligibility")
	}
	var state, reason jsontext.Value
	for _, member := range members {
		switch member.key {
		case "state":
			if state != nil {
				return nil, appError("duplicate field `state`")
			}
			state = member.value
		case "reason":
			if reason != nil {
				return nil, appError("duplicate field `reason`")
			}
			reason = member.value
		default:
			return nil, appError("unknown field `" + member.key + "`, expected `state` or `reason`")
		}
	}
	if state == nil {
		return nil, appError("missing field `state`")
	}
	tag, isString := jsonString(state)
	switch {
	case isString && tag == "composer_empty":
		if reason != nil && !jsonIsNull(reason) {
			return nil, appError("invalid type: expected unit variant InputEligibility::ComposerEmpty")
		}
		return inputEligibilityComposerEmpty{}, nil
	case isString && tag == "hold":
		if reason == nil {
			return nil, appError("missing field `reason`")
		}
		text, isString := jsonString(reason)
		if !isString {
			return nil, appError("invalid type: expected a GateReason string")
		}
		var parsed gateReason
		if err := parsed.UnmarshalText([]byte(text)); err != nil {
			return nil, err
		}
		return inputEligibilityHold{reason: parsed}, nil
	default:
		return nil, appError("unknown variant, expected `composer_empty` or `hold`")
	}
}

type transitionDraft struct {
	runID          int64
	episodeSeq     opt[int64]
	kind           runtimeEventKind
	classification opt[interactionClass]
	observedAt     string
	evidenceJSON   jsontext.Value
	closeReason    opt[interactionCloseReason]
	// causeJSON is nil when absent.
	causeJSON jsontext.Value
}

func (d transitionDraft) equal(other transitionDraft) bool {
	return d.runID == other.runID && d.episodeSeq == other.episodeSeq && d.kind == other.kind &&
		d.classification == other.classification && d.observedAt == other.observedAt &&
		jsonEqual(d.evidenceJSON, other.evidenceJSON) && d.closeReason == other.closeReason &&
		jsonOptionalEqual(d.causeJSON, other.causeJSON)
}

type episodeUpdate struct {
	visibility   visibility
	eligibility  inputEligibility
	transitions  []transitionDraft
	nextDeadline opt[time.Time]
}

func newEpisodeUpdate(v visibility, eligibility inputEligibility) episodeUpdate {
	return episodeUpdate{visibility: v, eligibility: eligibility, transitions: []transitionDraft{}, nextDeadline: none[time.Time]()}
}

func earliest(deadline *opt[time.Time], until time.Time) {
	if old, set := deadline.get(); set && old.Before(until) {
		return
	}
	*deadline = some(until)
}

// settled reports whether settle has passed since since; otherwise it schedules that instant as a
// deadline.
func settled(now, since time.Time, settle time.Duration, nextDeadline *opt[time.Time]) bool {
	if max(now.Sub(since), 0) >= settle {
		return true
	}
	earliest(nextDeadline, since.Add(settle))
	return false
}

func wallTime(value string) error {
	parsed, ok := parseRFC3339(value)
	if !ok {
		return runtimeError("observation requires RFC3339 UTC time")
	}
	_, offset := parsed.Zone()
	return ensure(offset == 0, "observation requires UTC time")
}

// materialize is draft as event sequence; the commit transaction accepts only the run's next
// sequence.
func materialize(draft transitionDraft, sequence int64) (observedTransition, error) {
	episode, episodic := draft.episodeSeq.get()
	_, classified := draft.classification.get()
	valid := draft.runID > 0 && sequence > 0 && episodic == classified && (!episodic || episode > 0)
	if err := ensure(valid, "valid transition identity required"); err != nil {
		return observedTransition{}, err
	}
	if err := wallTime(draft.observedAt); err != nil {
		return observedTransition{}, err
	}
	return observedTransition{
		RunID:          draft.runID,
		EventSeq:       sequence,
		EpisodeSeq:     draft.episodeSeq,
		Kind:           draft.kind,
		Classification: draft.classification,
		ObservedAt:     draft.observedAt,
		EvidenceJSON:   draft.evidenceJSON,
		CloseReason:    draft.closeReason,
		CauseJSON:      draft.causeJSON,
	}, nil
}

// observationHealth is a runner's input health as its control endpoint reports it; its JSON
// denies unknown members.
type observationHealth struct {
	Visibility       visibility
	Eligibility      inputEligibility
	PendingCount     uint64
	GapCount         uint64
	Degraded         bool
	Diagnostic       opt[string]
	WriterDiagnostic opt[string]
	GapOpenSince     opt[string]
}

type observationHealthWire struct {
	Visibility       visibility     `json:"visibility"`
	Eligibility      jsontext.Value `json:"eligibility"`
	PendingCount     uint64         `json:"pending_count"`
	GapCount         uint64         `json:"gap_count"`
	Degraded         bool           `json:"degraded"`
	Diagnostic       opt[string]    `json:"diagnostic"`
	WriterDiagnostic opt[string]    `json:"writer_diagnostic"`
	GapOpenSince     opt[string]    `json:"gap_open_since"`
}

func (h observationHealth) MarshalJSONTo(encoder *jsontext.Encoder) error {
	return json.MarshalEncode(encoder, observationHealthWire{
		Visibility:       h.Visibility,
		Eligibility:      jsonValue(inputEligibilityJSON(h.Eligibility)),
		PendingCount:     h.PendingCount,
		GapCount:         h.GapCount,
		Degraded:         h.Degraded,
		Diagnostic:       h.Diagnostic,
		WriterDiagnostic: h.WriterDiagnostic,
		GapOpenSince:     h.GapOpenSince,
	})
}

func (h *observationHealth) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	value, err := decoder.ReadValue()
	if err != nil {
		return err
	}
	var wire observationHealthWire
	if err := decodeStrict(value, &wire); err != nil {
		return err
	}
	eligibility, err := inputEligibilityDecode(wire.Eligibility)
	if err != nil {
		return err
	}
	*h = observationHealth{
		Visibility:       wire.Visibility,
		Eligibility:      eligibility,
		PendingCount:     wire.PendingCount,
		GapCount:         wire.GapCount,
		Degraded:         wire.Degraded,
		Diagnostic:       wire.Diagnostic,
		WriterDiagnostic: wire.WriterDiagnostic,
		GapOpenSince:     wire.GapOpenSince,
	}
	return nil
}

func (h observationHealth) validate() error {
	if err := ensure(h.PendingCount <= 130, "health pending count exceeds capacity"); err != nil {
		return err
	}
	if err := ensure(len(jsonValue(h)) <= 2048, "health observation exceeds encoded limit"); err != nil {
		return err
	}
	if text, set := h.WriterDiagnostic.get(); set {
		if err := ensure(h.Degraded && isDiagnosticText(text), "invalid health writer diagnostic"); err != nil {
			return err
		}
	}
	if started, set := h.GapOpenSince.get(); set {
		gapHold := h.Eligibility == inputEligibility(inputEligibilityHold{reason: gateReasonGap})
		if err := ensure(h.Degraded && gapHold, "health gap start outside a degraded gap hold"); err != nil {
			return err
		}
		if err := wallTime(started); err != nil {
			return err
		}
	}
	if code, set := h.Diagnostic.get(); set {
		_, invalid := observationFailureTryFrom(code)
		if err := ensure(invalid == nil, "invalid health diagnostic"); err != nil {
			return err
		}
	}
	_, diagnosed := h.Diagnostic.get()
	if err := ensure(!h.Degraded || diagnosed, "degraded health omits its diagnostic"); err != nil {
		return err
	}
	if h.Eligibility == inputEligibility(inputEligibilityComposerEmpty{}) {
		consistent := h.Visibility == visibilityNotVisible && !h.Degraded && h.PendingCount == 0
		if err := ensure(consistent, "health composer proof conflicts with held state"); err != nil {
			return err
		}
	}
	return nil
}

// captureTrigger names why a test native-capture port records a screen.
type captureTrigger uint8

const (
	captureTriggerChildOutput captureTrigger = iota + 1
	captureTriggerResize
	captureTriggerTimer
)

// observationCapture is the test-only native capture port; production controllers have none.
type observationCapture interface {
	observe(screen *screenSnapshot, trigger captureTrigger, c clock) error
	cancelOwnerExit() error
	beginShutdown(until time.Time)
}

type pendingEpisodeUpdate struct {
	update    episodeUpdate
	nextDraft int
	// materialized is the next draft as an event, nil until it is materialized.
	materialized *observedTransition
}

type pendingGap struct {
	startedAt string
	count     uint64
	reason    observationFailure
}

type pendingExit struct {
	exit      childExit
	at        string
	screenSeq uint64
}

// observationController turns screens into episode transitions and input health; one runner
// goroutine owns it, and only publishDeliveryResult may be called from another.
type observationController struct {
	run                      runRecord
	detector                 *detector
	tracker                  *episodeTracker
	nextEventSeq             int64
	pending                  *pendingEpisodeUpdate
	gap                      *pendingGap
	gapCount                 uint64
	terminalFenced           bool
	terminalObservationLoss  uint64
	diagnostic               opt[string]
	visibility               visibility
	eligibility              inputEligibility
	nextDeadline             opt[time.Time]
	started                  bool
	exit                     *pendingExit
	exitRecorded             childExit
	lastTime                 opt[time.Time]
	lastOutput               opt[uint64]
	catalog                  *profileCatalog
	requestGeneration        uint64
	requestGenerationFailed  bool
	currentRequest           *observedRequest
	currentContinuation      *observedContinuation
	transitRequest           *observedRequest
	requestRegion            *requestRegion
	currentClassification    opt[interactionClass]
	unplannedQuestionEpisode opt[int64]
	pendingErr               *cafleetError
	deliveryMu               sync.Mutex
	deliveryResults          []jsontext.Value
	diagnostics              runnerDiagnostics
	capture                  observationCapture
	observationHooks         observationHooks
}

type requestRegion struct {
	top  int
	rows []string
}

func screenRows(screen *screenSnapshot) []string {
	return screen.screen.Rows(0, screen.cols)
}

func requestRegionOf(screen *screenSnapshot, top, bottom uint16) *requestRegion {
	rows := screenRows(screen)
	start := min(int(top), len(rows))
	end := min(int(bottom), len(rows))
	return &requestRegion{top: int(top), rows: slices.Clone(rows[start:max(start, end)])}
}

func (r *requestRegion) unchanged(screen *screenSnapshot) bool {
	rows := screenRows(screen)
	start := min(r.top, len(rows))
	end := min(r.top+len(r.rows), len(rows))
	return slices.Equal(rows[start:end], r.rows)
}

// observedRequest is the current answerable request; it has no debug or serialized form.
type observedRequest struct {
	requestGeneration uint64
	answerMatches     []answerMatch
	episodeSeq        int64
	classification    interactionClass
	requestRuleID     string
	semantic          semanticFingerprint
	choices           []string
	focus             string
	outputSeq         uint64
}

func (observedRequest) String() string             { return redacted }
func (observedRequest) GoString() string           { return redacted }
func (observedRequest) Format(f fmt.State, _ rune) { writeRedacted(f) }
func (observedRequest) MarshalJSON() ([]byte, error) {
	return nil, errors.New("observed requests are not serializable")
}

func (r *observedRequest) sameRequest(other *observedRequest) bool {
	return r.episodeSeq == other.episodeSeq && r.classification == other.classification &&
		r.requestRuleID == other.requestRuleID && r.semantic.equal(&other.semantic) &&
		slices.Equal(r.choices, other.choices) && r.focus == other.focus &&
		slices.EqualFunc(r.answerMatches, other.answerMatches, func(a, b answerMatch) bool { return a.sameRequest(&b) })
}

type observedContinuation struct {
	requestGeneration uint64
	episodeSeq        int64
	ruleID            string
	fingerprint       semanticFingerprint
}

func (c *observedContinuation) equal(other *observedContinuation) bool {
	return c.requestGeneration == other.requestGeneration && c.episodeSeq == other.episodeSeq &&
		c.ruleID == other.ruleID && c.fingerprint.equal(&other.fingerprint)
}

// orderedObservation observes the screens of one transport pass through its controller.
type orderedObservation struct {
	controller *observationController
	writer     *eventWriter
	clock      clock
	phase      observationPhase
}

func (o *orderedObservation) inputHealth() observationHealth {
	return o.controller.health(o.writer)
}

func (o *orderedObservation) observe(screen *screenSnapshot) error {
	return o.controller.observeScreen(screen, o.writer, o.clock, o.phase)
}

func (o *orderedObservation) parserFailure() error {
	return o.controller.observerFailed(observationFailureParser, o.clock)
}

func newObservationController(run runRecord, catalog *profileCatalog) (*observationController, error) {
	_, child := run.childPID.get()
	_, childStart := run.childStartIdentity.get()
	_, started := run.startedAt.get()
	_, ended := run.endedAt.get()
	_, recovered := run.recoveryOriginStatus.get()
	fresh := run.runID > 0 && run.ownerEpoch > 0 && run.ownerKind == ownerKindRunner &&
		run.status == runStatusStarting && !child && !childStart && !started && !ended && !recovered
	if err := ensure(fresh, "observation controller requires a fresh starting run"); err != nil {
		return nil, err
	}
	backend, known := codingAgentCodingAgent(run.backend)
	if !known {
		return nil, runtimeError("unknown observer backend")
	}
	if err := ensure(catalog.detectionCoverage(backend) == run.coverage, "observer coverage mismatch"); err != nil {
		return nil, err
	}
	tracker, err := newEpisodeTracker(run.runID)
	if err != nil {
		return nil, err
	}
	return &observationController{
		run:                      run,
		detector:                 newDetector(catalog, backend, run.nativeVersion),
		tracker:                  tracker,
		nextEventSeq:             1,
		diagnostic:               none[string](),
		visibility:               visibilityUnknown,
		eligibility:              inputEligibilityHold{reason: gateReasonUnknown},
		nextDeadline:             none[time.Time](),
		lastTime:                 none[time.Time](),
		lastOutput:               none[uint64](),
		catalog:                  catalog,
		currentClassification:    none[interactionClass](),
		unplannedQuestionEpisode: none[int64](),
		diagnostics:              newRunnerDiagnostics(),
	}, nil
}

// shareDiagnostics records gap openings into the runner's shared diagnostics buffer, which the
// event writer flushes; until called, they go to a buffer of this controller alone.
func (c *observationController) shareDiagnostics(diagnostics runnerDiagnostics) {
	c.diagnostics = diagnostics
}

// publishDeliveryResult queues a deferred input's first result for publication on the next
// poll, once per input.
func (c *observationController) publishDeliveryResult(record jsontext.Value) {
	c.deliveryMu.Lock()
	defer c.deliveryMu.Unlock()
	for _, queued := range c.deliveryResults {
		duplicate := true
		for _, key := range []string{"input_id", "message_id", "blocked_by_input_id", "kind"} {
			duplicate = duplicate && jsonEqual(jsonIndex(queued, key), jsonIndex(record, key))
		}
		if duplicate {
			return
		}
	}
	c.deliveryResults = append(c.deliveryResults, record)
}

func (c *observationController) publishDeliveryStalled(record jsontext.Value) {
	c.publishDeliveryResult(record)
}

func (c *observationController) activeEpisodeSeq() opt[int64] {
	if episode, set := c.tracker.activeEpisode().get(); set {
		return some(episode.sequence)
	}
	return none[int64]()
}

func (c *observationController) currentEvidence() jsontext.Value {
	return c.tracker.currentEvidence
}

func (c *observationController) invalidateRequest() error {
	c.currentRequest = nil
	c.currentContinuation = nil
	c.transitRequest = nil
	c.requestRegion = nil
	if c.requestGenerationFailed || c.requestGeneration == math.MaxUint64 {
		c.requestGenerationFailed = true
		return runtimeError("permission request generation exhausted")
	}
	c.requestGeneration++
	return nil
}

func (c *observationController) nextRequest(screen *screenSnapshot, observed *screenObservation) (*observedRequest, opt[int64], error) {
	unplanned := none[int64]()
	class, interaction := observed.kind.interaction()
	answerable := interaction && (class == interactionClassPermissionRequest || class == interactionClassQuestion)
	if !answerable || observed.matchEvidence != matchEvidenceComplete ||
		c.eligibility != inputEligibility(inputEligibilityHold{reason: gateReasonInteraction}) {
		return nil, unplanned, nil
	}
	episode, active := c.tracker.activeEpisode().get()
	if !active {
		return nil, unplanned, nil
	}
	backend, known := codingAgentCodingAgent(c.run.backend)
	if !known {
		panic("validated observer backend")
	}
	plans := c.catalog.answerPlans(backend, observed.ruleID)
	if len(plans) == 0 && episode.class == interactionClassQuestion {
		unplanned = some(episode.sequence)
	}
	matches, err := c.detector.matchAnswers(screen, observed, plans)
	if err != nil {
		return nil, unplanned, err
	}
	if len(matches) == 0 {
		return nil, unplanned, nil
	}
	first := matches[0]
	return &observedRequest{
		requestGeneration: c.requestGeneration,
		episodeSeq:        episode.sequence,
		classification:    episode.class,
		requestRuleID:     observed.ruleID,
		semantic:          first.semantic,
		choices:           slices.Clone(first.choices),
		focus:             first.focus,
		outputSeq:         first.outputSeq,
		answerMatches:     matches,
	}, unplanned, nil
}

func (c *observationController) updateRequest(screen *screenSnapshot, observed *screenObservation) error {
	if err := ensure(!c.requestGenerationFailed, "permission request generation exhausted"); err != nil {
		return err
	}
	c.currentClassification = none[interactionClass]()
	if class, interaction := observed.kind.interaction(); interaction {
		c.currentClassification = some(class)
	}
	next, unplanned, err := c.nextRequest(screen, observed)
	if err != nil {
		return err
	}
	c.unplannedQuestionEpisode = unplanned
	if next == nil && c.cursorTransit(screen, observed) {
		if c.currentRequest != nil {
			c.transitRequest = c.currentRequest
			c.currentRequest = nil
		}
		return nil
	}
	resumed := c.transitRequest
	c.transitRequest = nil
	old := c.currentRequest
	if old == nil {
		old = resumed
	}
	same := (old == nil && next == nil) || (old != nil && next != nil && old.sameRequest(next))
	if !same {
		if err := c.invalidateRequest(); err != nil {
			return err
		}
		if next != nil {
			region, set := observed.matchedRegion.get()
			if !set {
				panic("a complete permission match has its live region")
			}
			c.requestRegion = requestRegionOf(screen, region.top, region.bottom)
		}
	}
	if next != nil {
		next.requestGeneration = c.requestGeneration
	}
	c.currentRequest = next
	c.currentContinuation = nil
	if episode, active := c.tracker.activeEpisode().get(); c.currentRequest == nil && active && c.tracker.continuesActive(observed) {
		if observed.fingerprint == nil {
			panic("a continuation observation has its fingerprint")
		}
		c.currentContinuation = &observedContinuation{
			requestGeneration: c.requestGeneration,
			episodeSeq:        episode.sequence,
			ruleID:            observed.ruleID,
			fingerprint:       *observed.fingerprint,
		}
	}
	return nil
}

func (c *observationController) cursorTransit(screen *screenSnapshot, observed *screenObservation) bool {
	_, interaction := observed.kind.interaction()
	return (c.currentRequest != nil || c.transitRequest != nil) && !observed.verifiedNoninteraction && !interaction &&
		(c.pending == nil || len(c.pending.update.transitions) == 0) &&
		c.requestRegion != nil && c.requestRegion.unchanged(screen)
}

func (c *observationController) moment(cl clock) (observationMoment, error) {
	now := observationMomentSample(cl)
	if old, set := c.lastTime.get(); set && now.monotonic.Before(old) {
		return observationMoment{}, runtimeError("observation clock regressed")
	}
	c.lastTime = some(now.monotonic)
	return now, nil
}

func (c *observationController) validateOutput(screen *screenSnapshot) error {
	if old, set := c.lastOutput.get(); set && screen.screenSeq < old {
		return runtimeError("observation output sequence regressed")
	}
	c.lastOutput = some(screen.screenSeq)
	return nil
}

func (c *observationController) lifecycle(kind runtimeEventKind, cause jsontext.Value, at string, screenSeq uint64) (transitionDraft, error) {
	evidence, err := runEvidence(c.run, "runner-lifecycle", screenSeq)
	if err != nil {
		return transitionDraft{}, err
	}
	return transitionDraft{
		runID:          c.run.runID,
		episodeSeq:     none[int64](),
		kind:           kind,
		classification: none[interactionClass](),
		observedAt:     at,
		evidenceJSON:   evidence,
		closeReason:    none[interactionCloseReason](),
		causeJSON:      cause,
	}, nil
}

func (c *observationController) retain(update episodeUpdate) error {
	if err := ensure(c.pending == nil && len(update.transitions) <= 2, "one bounded observation update required"); err != nil {
		return err
	}
	c.visibility = update.visibility
	c.eligibility = update.eligibility
	c.nextDeadline = update.nextDeadline
	if len(update.transitions) > 0 {
		c.pending = &pendingEpisodeUpdate{update: update}
	}
	return nil
}

func (c *observationController) update(transitions []transitionDraft) episodeUpdate {
	update := newEpisodeUpdate(c.visibility, c.eligibility)
	update.transitions = transitions
	update.nextDeadline = c.nextDeadline
	return update
}

func (c *observationController) recordGap(reason observationFailure, now observationMoment) error {
	if err := c.invalidateRequest(); err != nil {
		return err
	}
	c.currentClassification = none[interactionClass]()
	c.unplannedQuestionEpisode = none[int64]()
	if c.gapCount == math.MaxUint64 {
		return runtimeError("observation gap count overflow")
	}
	c.gapCount++
	c.diagnostic = some(reason.asStr())
	c.visibility = visibilityUnknown
	c.eligibility = inputEligibilityHold{reason: gateReasonGap}
	c.nextDeadline = none[time.Time]()
	if c.terminalFenced {
		if c.terminalObservationLoss == math.MaxUint64 {
			return runtimeError("terminal observation loss overflow")
		}
		c.terminalObservationLoss++
		return nil
	}
	if c.gap == nil {
		c.diagnostics.record(runnerDiagnosticSourceObservation, "observation gap opened: "+reason.asStr(), now.observedAt)
		c.gap = &pendingGap{startedAt: now.observedAt, reason: reason}
	}
	if c.gap.count == math.MaxUint64 {
		return runtimeError("gap interval count overflow")
	}
	c.gap.count++
	return c.tracker.resetObservation()
}

func (c *observationController) deliveryPressure(reason observationFailure, now observationMoment) error {
	if c.terminalFenced {
		c.eligibility = inputEligibilityHold{reason: gateReasonGap}
		c.nextDeadline = none[time.Time]()
		return nil
	}
	return c.recordGap(reason, now)
}

func (c *observationController) route(writer *eventWriter, now observationMoment) error {
	for c.pending != nil {
		pending := c.pending
		if pending.nextDraft >= len(pending.update.transitions) {
			c.pending = nil
			c.pendingErr = nil
			continue
		}
		if pending.materialized == nil {
			materialized, err := materialize(pending.update.transitions[pending.nextDraft], c.nextEventSeq)
			if err != nil {
				return err
			}
			pending.materialized = &materialized
		}
		value := *pending.materialized
		pending.materialized = nil
		retained := writer.trySubmit(value)
		switch {
		case retained == nil:
			pending.nextDraft++
			c.nextEventSeq++
		case retained.reason == retainReasonEvidenceInvalid:
			if isEventRejection(retained.transition) {
				panic("a rejection event always carries valid run evidence")
			}
			screenSeq, set := c.lastOutput.get()
			if !set {
				panic("an evidence refusal follows a validated screen")
			}
			c.diagnostics.record(runnerDiagnosticSourceEventWriter,
				fmt.Sprintf("event writer rejected %s event %d: %s", retained.transition.Kind.asStr(), retained.transition.EventSeq, retained.cause.message()),
				now.observedAt)
			replacement, err := eventRejectedTransition(c.run, retained.transition, screenSeq, retained.cause, now.observedAt)
			if err != nil {
				return err
			}
			pending.materialized = &replacement
		default:
			transition := retained.transition
			pending.materialized = &transition
			c.pendingErr = retained.cause
			return c.deliveryPressure(observationFailureSink, now)
		}
	}
	return nil
}

func (c *observationController) start(run runRecord, current *screenSnapshot, writer *eventWriter, cl clock) error {
	_, child := run.childPID.get()
	_, childStart := run.childStartIdentity.get()
	_, started := run.startedAt.get()
	matching := !c.started && run.owner().equal(c.run.owner()) && run.launchNonce == c.run.launchNonce &&
		jsonEqual(run.databaseIdentityJSON, c.run.databaseIdentityJSON) && run.backend == c.run.backend &&
		run.nativeVersion == c.run.nativeVersion && run.status == runStatusRunning && child && childStart && started
	if err := ensure(matching, "started requires matching committed native child"); err != nil {
		return err
	}
	c.started = true
	c.run = run
	now, err := c.moment(cl)
	if err != nil {
		return err
	}
	if run.coverage == coverageUnknown {
		value, err := c.lifecycle(runtimeEventKindCoverageUnknown, jsonValue(jsonObject{{"kind", "coverage_unknown"}}), now.observedAt, current.screenSeq)
		if err != nil {
			return err
		}
		if err := c.retain(c.update([]transitionDraft{value})); err != nil {
			return err
		}
		return c.route(writer, now)
	}
	return nil
}

func (c *observationController) observer(writer *eventWriter, cl clock) *orderedObservation {
	return &orderedObservation{controller: c, writer: writer, clock: cl, phase: observationPhaseChildOutput}
}

// observation is the detector's reading of screen, or false after recording a detector gap.
func (c *observationController) observation(screen *screenSnapshot, now observationMoment, phase observationPhase, cl clock) (screenObservation, bool, error) {
	failed := c.observationHooks != nil && c.observationHooks.before(phase, observationOperationDetector) != nil
	if phase == observationPhaseResize && c.capture != nil {
		if err := c.capture.observe(screen, captureTriggerResize, cl); err != nil {
			return screenObservation{}, false, err
		}
	}
	if failed {
		return screenObservation{}, false, c.recordGap(observationFailureDetector, now)
	}
	observed, err := c.detector.observe(screen)
	if err != nil {
		return screenObservation{}, false, c.recordGap(observationFailureDetector, now)
	}
	return observed, true, nil
}

func (c *observationController) observeScreen(screen *screenSnapshot, writer *eventWriter, cl clock, phase observationPhase) error {
	now, err := c.moment(cl)
	if err != nil {
		return err
	}
	if err := c.validateOutput(screen); err != nil {
		return err
	}
	if phase != observationPhaseResize && c.capture != nil {
		if err := c.capture.observe(screen, captureTriggerChildOutput, cl); err != nil {
			return err
		}
	}
	if !c.started || c.exitRecorded != nil {
		return nil
	}
	if _, failing := writer.status().diagnostic.get(); c.pending != nil || c.gap != nil || failing {
		return c.recordGap(observationFailureSink, now)
	}
	observed, ok, err := c.observation(screen, now, phase, cl)
	if err != nil || !ok {
		return err
	}
	return c.apply(screen, &observed, writer, now)
}

func (c *observationController) apply(screen *screenSnapshot, observed *screenObservation, writer *eventWriter, now observationMoment) error {
	update, err := c.tracker.observe(observed, now.monotonic, now.observedAt)
	if err != nil {
		return err
	}
	if err := c.retain(update); err != nil {
		return err
	}
	if err := c.updateRequest(screen, observed); err != nil {
		return err
	}
	return c.route(writer, now)
}

// quiet reports that no retained transition, unsaved gap or degraded writer holds new
// observation work back.
func (c *observationController) quiet(writer *eventWriter) bool {
	_, failing := writer.status().diagnostic.get()
	return c.pending == nil && c.gap == nil && !failing
}

func (c *observationController) poll(current *screenSnapshot, writer *eventWriter, cl clock) (observationStatus, error) {
	return c.pollAt(current, writer, cl, observationPhaseQuietTimer)
}

func (c *observationController) takeDeliveryResult() (jsontext.Value, bool) {
	c.deliveryMu.Lock()
	defer c.deliveryMu.Unlock()
	if len(c.deliveryResults) == 0 {
		return nil, false
	}
	record := c.deliveryResults[0]
	c.deliveryResults = c.deliveryResults[1:]
	return record, true
}

func (c *observationController) deliveryResultsEmpty() bool {
	c.deliveryMu.Lock()
	defer c.deliveryMu.Unlock()
	return len(c.deliveryResults) == 0
}

func gapCause(kind runtimeCauseKind, gap *pendingGap, endedAt string) jsontext.Value {
	ended := none[string]()
	if kind == runtimeCauseKindStorageRecovered {
		ended = some(endedAt)
	}
	return jsonValue(jsonObject{{"kind", kind.asStr()}, {"gap_count", gap.count}, {"gap_started_at", gap.startedAt}, {"gap_ended_at", ended}})
}

func (c *observationController) pollAt(current *screenSnapshot, writer *eventWriter, cl clock, phase observationPhase) (observationStatus, error) {
	now, err := c.moment(cl)
	if err != nil {
		return observationStatus{}, err
	}
	if err := c.validateOutput(current); err != nil {
		return observationStatus{}, err
	}
	if c.capture != nil {
		if err := c.capture.observe(current, captureTriggerTimer, cl); err != nil {
			return observationStatus{}, err
		}
	}
	if _, failing := writer.status().diagnostic.get(); !c.terminalFenced && failing && c.gap == nil {
		if err := c.recordGap(observationFailureSink, now); err != nil {
			return observationStatus{}, err
		}
	}
	if err := c.route(writer, now); err != nil {
		return observationStatus{}, err
	}
	if c.pending != nil {
		return c.status(), nil
	}
	if _, failing := writer.status().diagnostic.get(); c.gap != nil && !failing {
		gap := c.gap
		c.gap = nil
		firstKind := runtimeCauseKindStoragePressure
		if gap.reason == observationFailureParser || gap.reason == observationFailureDetector {
			firstKind = runtimeCauseKindObserverError
		}
		first, err := c.lifecycle(runtimeEventKindRunnerDegraded, gapCause(firstKind, gap, now.observedAt), gap.startedAt, current.screenSeq)
		if err != nil {
			return observationStatus{}, err
		}
		last, err := c.lifecycle(runtimeEventKindRunnerDegraded, gapCause(runtimeCauseKindStorageRecovered, gap, now.observedAt), now.observedAt, current.screenSeq)
		if err != nil {
			return observationStatus{}, err
		}
		if err := c.retain(c.update([]transitionDraft{first, last})); err != nil {
			return observationStatus{}, err
		}
		if err := c.route(writer, now); err != nil {
			return observationStatus{}, err
		}
		if c.pending == nil {
			c.diagnostic = none[string]()
			c.nextDeadline = some(now.monotonic)
		}
	}
	if c.started && !c.terminalFenced && c.quiet(writer) {
		if record, queued := c.takeDeliveryResult(); queued {
			kindText, isString := jsonString(jsonIndex(record, "kind"))
			if !isString {
				panic("a delivery record names its event kind")
			}
			kind, err := runtimeEventKindTryFrom(kindText)
			if err != nil {
				return observationStatus{}, err
			}
			if err := ensure(isDeliveryEvent(kind), "delivery publication requires a delivery event"); err != nil {
				return observationStatus{}, err
			}
			value, err := c.lifecycle(kind, record, now.observedAt, current.screenSeq)
			if err != nil {
				return observationStatus{}, err
			}
			if err := c.retain(c.update([]transitionDraft{value})); err != nil {
				return observationStatus{}, err
			}
			if err := c.route(writer, now); err != nil {
				return observationStatus{}, err
			}
		}
	}
	if c.quiet(writer) && c.deliveryResultsEmpty() && c.exit != nil {
		exit := c.exit
		c.exit = nil
		c.terminalFenced = true
		update, err := c.tracker.finish(exit.at)
		if err != nil {
			return observationStatus{}, err
		}
		var cause jsonObject
		switch native := exit.exit.(type) {
		case childExitCode:
			cause = jsonObject{{"kind", runtimeCauseKindNativeExit.asStr()}, {"exit_code", native.code}, {"exit_signal", nil}}
		case childExitSignal:
			cause = jsonObject{{"kind", runtimeCauseKindNativeExit.asStr()}, {"exit_code", nil}, {"exit_signal", native.signal}}
		default:
			panic("invalid childExit")
		}
		exited, err := c.lifecycle(runtimeEventKindRunnerExited, jsonValue(cause), exit.at, exit.screenSeq)
		if err != nil {
			return observationStatus{}, err
		}
		update.transitions = append(update.transitions, exited)
		if err := c.retain(update); err != nil {
			return observationStatus{}, err
		}
		if err := c.route(writer, now); err != nil {
			return observationStatus{}, err
		}
	}
	if deadline, set := c.nextDeadline.get(); c.started && c.exitRecorded == nil && c.quiet(writer) && set && !now.monotonic.Before(deadline) {
		value, ok, err := c.observation(current, now, phase, cl)
		if err != nil {
			return observationStatus{}, err
		}
		if !ok {
			return c.status(), nil
		}
		if err := c.apply(current, &value, writer, now); err != nil {
			return observationStatus{}, err
		}
	}
	return c.status(), nil
}

func (c *observationController) childExited(exit childExit, current *screenSnapshot, writer *eventWriter, cl clock) error {
	if c.exitRecorded != nil {
		if err := ensure(c.exitRecorded == exit, "native exit identity changed"); err != nil {
			return err
		}
	} else {
		now, err := c.moment(cl)
		if err != nil {
			return err
		}
		if err := c.invalidateRequest(); err != nil {
			return err
		}
		c.exitRecorded = exit
		c.exit = &pendingExit{exit: exit, at: now.observedAt, screenSeq: current.screenSeq}
	}
	if c.capture != nil {
		if err := c.capture.cancelOwnerExit(); err != nil {
			return err
		}
	}
	_, err := c.poll(current, writer, cl)
	return err
}

func (c *observationController) observerFailed(reason observationFailure, cl clock) error {
	now, err := c.moment(cl)
	if err != nil {
		return err
	}
	return c.recordGap(reason, now)
}

// pendingError is the writer refusal that holds the retained transition, nil when none does.
func (c *observationController) pendingError() *cafleetError {
	return c.pendingErr
}

func (c *observationController) pendingTransitionCount() uint64 {
	if c.pending == nil {
		return 0
	}
	return uint64(len(c.pending.update.transitions) - c.pending.nextDraft) //nolint:gosec // nextDraft never passes the transition count
}

func (c *observationController) unsavedGapCount() uint64 {
	count := uint64(0)
	if c.gap != nil {
		count = c.gap.count
	}
	return checkedAddUint64(count, c.terminalObservationLoss, "unsaved observation count")
}

func (c *observationController) hasPending() bool {
	return c.status().retainedDraft || c.exit != nil || c.gap != nil
}

func (c *observationController) status() observationStatus {
	return observationStatus{
		visibility:         c.visibility,
		eligibility:        c.eligibility,
		nextDeadline:       c.nextDeadline,
		retainedTransition: c.pending != nil && c.pending.materialized != nil,
		retainedDraft:      c.pending != nil && c.pending.nextDraft < len(c.pending.update.transitions),
		gapCount:           c.gapCount,
		diagnostic:         c.diagnostic,
	}
}

func (c *observationController) health(writer *eventWriter) observationHealth {
	observer := c.status()
	state := writer.status()
	pending := c.pendingTransitionCount()
	writerDiagnostic, failing := state.diagnostic.get()
	_, diagnosed := observer.diagnostic.get()
	degraded := failing || c.gap != nil || diagnosed
	eligibility := observer.eligibility
	if degraded || pending+state.pendingCount > 0 {
		eligibility = inputEligibilityHold{reason: gateReasonGap}
	}
	health := observationHealth{
		Visibility:       observer.visibility,
		Eligibility:      eligibility,
		PendingCount:     pending + state.pendingCount,
		GapCount:         checkedAddUint64(observer.gapCount, state.gapCount, "health gap count"),
		Degraded:         degraded,
		Diagnostic:       observer.diagnostic,
		WriterDiagnostic: none[string](),
		GapOpenSince:     none[string](),
	}
	if failing {
		health.Diagnostic = some("sink_unavailable")
		health.WriterDiagnostic = some(diagnosticText(writerDiagnostic))
	}
	if c.gap != nil {
		health.GapOpenSince = some(c.gap.startedAt)
	}
	return health
}

type candidateFingerprint struct {
	fingerprint semanticFingerprint
	since       time.Time
}

// candidateSince is when fingerprint was first seen, restarting the candidate when the
// fingerprint changes.
func candidateSince(slot **candidateFingerprint, fingerprint *semanticFingerprint, now time.Time) time.Time {
	if *slot != nil && (*slot).fingerprint.equal(fingerprint) {
		return (*slot).since
	}
	*slot = &candidateFingerprint{fingerprint: *fingerprint, since: now}
	return now
}

type activeEpisode struct {
	sequence    int64
	class       interactionClass
	ruleID      string
	fingerprint semanticFingerprint
	evidence    jsontext.Value
}

// episodeRef is an episode's sequence and classification.
type episodeRef struct {
	sequence int64
	class    interactionClass
}

type episodeTracker struct {
	runID       int64
	nextEpisode opt[int64]
	active      *activeEpisode
	candidate   *candidateFingerprint
	boundary    opt[time.Time]
	lastOutput  opt[uint64]
	lastTime    opt[time.Time]
	finished    bool
	// currentEvidence is nil before the first observation and after a reset.
	currentEvidence jsontext.Value
}

const (
	candidateSettle = 100 * time.Millisecond
	boundarySettle  = 250 * time.Millisecond
)

func newEpisodeTracker(runID int64) (*episodeTracker, error) {
	if err := ensure(runID > 0, "positive run identity required"); err != nil {
		return nil, err
	}
	return &episodeTracker{runID: runID, nextEpisode: some(int64(1)), boundary: none[time.Time](), lastOutput: none[uint64](), lastTime: none[time.Time]()}, nil
}

func (t *episodeTracker) activeEpisode() opt[episodeRef] {
	if t.active == nil {
		return none[episodeRef]()
	}
	return some(episodeRef{sequence: t.active.sequence, class: t.active.class})
}

func (t *episodeTracker) continuesActive(value *screenObservation) bool {
	rule, continues := value.continuesRuleID.get()
	return value.kind == observationKindPermissionContinuation && value.matchEvidence == matchEvidenceComplete &&
		t.active != nil && t.active.class == interactionClassPermissionRequest && continues && rule == t.active.ruleID
}

func (t *episodeTracker) draft(kind runtimeEventKind, episode opt[episodeRef], evidence jsontext.Value, reason opt[interactionCloseReason], at string) transitionDraft {
	draft := transitionDraft{
		runID:          t.runID,
		episodeSeq:     none[int64](),
		classification: none[interactionClass](),
		kind:           kind,
		observedAt:     at,
		evidenceJSON:   evidence,
		closeReason:    reason,
	}
	if ref, set := episode.get(); set {
		draft.episodeSeq, draft.classification = some(ref.sequence), some(ref.class)
	}
	return draft
}

// close ends the active episode; false when no episode is active.
func (t *episodeTracker) close(reason interactionCloseReason, evidence jsontext.Value, at string) (transitionDraft, bool) {
	active := t.active
	if active == nil {
		return transitionDraft{}, false
	}
	t.active = nil
	if evidence == nil {
		evidence = active.evidence
	}
	return t.draft(runtimeEventKindInteractionClosed, some(episodeRef{sequence: active.sequence, class: active.class}), evidence, some(reason), at), true
}

func (t *episodeTracker) open(value *screenObservation, class interactionClass, evidence jsontext.Value, at string) (transitionDraft, error) {
	sequence, set := t.nextEpisode.get()
	if !set {
		return transitionDraft{}, runtimeError("episode sequence exhausted")
	}
	t.nextEpisode = none[int64]()
	if sequence < math.MaxInt64 {
		t.nextEpisode = some(sequence + 1)
	}
	draft := t.draft(runtimeEventKindInteractionOpened, some(episodeRef{sequence: sequence, class: class}), evidence, none[interactionCloseReason](), at)
	if value.fingerprint == nil {
		return transitionDraft{}, runtimeError("interaction fingerprint required")
	}
	t.active = &activeEpisode{sequence: sequence, class: class, ruleID: value.ruleID, fingerprint: *value.fingerprint, evidence: evidence}
	t.candidate = nil
	return draft, nil
}

func (t *episodeTracker) observe(value *screenObservation, now time.Time, at string) (episodeUpdate, error) {
	if err := ensure(!t.finished, "closed tracker accepts no observations"); err != nil {
		return episodeUpdate{}, err
	}
	if err := wallTime(at); err != nil {
		return episodeUpdate{}, err
	}
	if old, set := t.lastOutput.get(); set && value.outputSeq < old {
		return episodeUpdate{}, runtimeError("observation output sequence regressed")
	}
	if old, set := t.lastTime.get(); set && now.Before(old) {
		return episodeUpdate{}, runtimeError("monotonic observation time regressed")
	}
	if err := ensure(value.rawEvidence.screenSeq == value.outputSeq, "observation evidence sequence mismatch"); err != nil {
		return episodeUpdate{}, err
	}
	evidence, err := metadataEvidence(jsonValue(value.rawEvidence))
	if err != nil {
		return episodeUpdate{}, err
	}
	t.currentEvidence = evidence
	class, interaction := value.kind.interaction()
	if interaction && value.fingerprint == nil {
		return episodeUpdate{}, runtimeError("semantic observation requires fingerprint")
	}
	t.lastOutput = some(value.outputSeq)
	t.lastTime = some(now)
	update := newEpisodeUpdate(visibilityUnknown, inputEligibilityHold{reason: gateReasonUnknown})
	if interaction {
		t.clearBoundaries()
		update.visibility = visibilityVisible
		if t.active != nil && t.active.fingerprint.equal(value.fingerprint) {
			t.candidate = nil
			update.eligibility = inputEligibilityHold{reason: gateReasonInteraction}
			return update, nil
		}
		since := candidateSince(&t.candidate, value.fingerprint, now)
		if !settled(now, since, candidateSettle, &update.nextDeadline) {
			update.eligibility = inputEligibilityHold{reason: gateReasonCandidate}
			return update, nil
		}
		if closed, set := t.close(interactionCloseReasonSuperseded, nil, at); set {
			update.transitions = append(update.transitions, closed)
		}
		opened, err := t.open(value, class, evidence, at)
		if err != nil {
			return episodeUpdate{}, err
		}
		update.transitions = append(update.transitions, opened)
		update.eligibility = inputEligibilityHold{reason: gateReasonInteraction}
		return update, nil
	}
	if t.continuesActive(value) {
		t.candidate = nil
		t.clearBoundaries()
		update.visibility = visibilityVisible
		update.eligibility = inputEligibilityHold{reason: gateReasonInteraction}
		return update, nil
	}
	t.candidate = nil
	if value.verifiedNoninteraction {
		if t.active != nil {
			since, set := t.boundary.get()
			if !set {
				since = now
				t.boundary = some(now)
			}
			if settled(now, since, boundarySettle, &update.nextDeadline) {
				if closed, set := t.close(interactionCloseReasonNotVisible, evidence, at); set {
					update.transitions = append(update.transitions, closed)
				}
				t.boundary = none[time.Time]()
			}
		}
	} else {
		t.boundary = none[time.Time]()
	}
	update.visibility = visibilityNotVisible
	if t.active != nil {
		update.visibility = visibilityObscured
	}
	switch {
	case value.kind == observationKindComposerEmpty && t.active != nil:
		update.eligibility = inputEligibilityHold{reason: gateReasonInteraction}
	case value.kind == observationKindComposerEmpty && value.matchEvidence == matchEvidenceComplete:
		update.eligibility = inputEligibilityComposerEmpty{}
	case t.active != nil:
		update.eligibility = inputEligibilityHold{reason: gateReasonInteraction}
	default:
		update.eligibility = inputEligibilityHold{reason: gateReasonUnknown}
	}
	return update, nil
}

func (t *episodeTracker) resetObservation() error {
	t.currentEvidence = nil
	t.candidate = nil
	t.clearBoundaries()
	return nil
}

func (t *episodeTracker) clearBoundaries() {
	t.boundary = none[time.Time]()
}

func (t *episodeTracker) finish(at string) (episodeUpdate, error) {
	if err := wallTime(at); err != nil {
		return episodeUpdate{}, err
	}
	update := newEpisodeUpdate(visibilityNotVisible, inputEligibilityHold{reason: gateReasonObserverUnavailable})
	if closed, set := t.close(interactionCloseReasonProcessExited, nil, at); set {
		update.transitions = append(update.transitions, closed)
	}
	t.finished = true
	t.candidate = nil
	return update, nil
}
