package core

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
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
