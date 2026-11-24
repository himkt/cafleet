package core

import (
	"bytes"
	"context"
	"encoding/json/jsontext"
	"fmt"
	"math"
	"slices"
	"strings"
	"time"
	"unicode/utf8"
)

const preSubmitWindow = time.Second

func inspectionAssociation(ctx context.Context, conn *dbConn, run runRecord, input permissionInspectInput) (runtimeEventView, error) {
	if err := ensure(
		input.EventID > 0 && input.InteractionID > 0 && input.DirectorMemberID == run.directorMemberID,
		"permission inspection identity mismatch",
	); err != nil {
		return runtimeEventView{}, err
	}
	view, err := getEvent(ctx, conn, input.EventID)
	if err != nil {
		return runtimeEventView{}, err
	}
	messageID, messaged := view.event.messageID.get()
	interaction, interacted := view.interaction.get()
	if err := ensure(
		view.event.runID == run.runID &&
			view.event.interactionID == some(input.InteractionID) &&
			view.event.kind == runtimeEventKindInteractionOpened &&
			view.event.notificationState == notificationStateInbox &&
			messaged && messageID > 0 &&
			interacted &&
			interaction.runID == run.runID &&
			(interaction.classification == interactionClassPermissionRequest || interaction.classification == interactionClassQuestion) &&
			!interaction.closedAt.set,
		"permission inspection current event/interaction/message mismatch",
	); err != nil {
		return runtimeEventView{}, err
	}
	director, err := getMember(ctx, conn, run.directorMemberID, run.fleetID)
	if err != nil {
		return runtimeEventView{}, err
	}
	record, found := director.get()
	if !found {
		return runtimeEventView{}, runtimeError("permission Director is absent")
	}
	fleet, err := activeMemberFleet(ctx, conn, run.directorMemberID)
	if err != nil {
		return runtimeEventView{}, err
	}
	if err := ensure(record.kind == memberKindDirector && fleet == some(run.fleetID), "permission Director is inactive"); err != nil {
		return runtimeEventView{}, err
	}
	live, err := queryRow(conn,
		"SELECT EXISTS(SELECT 1 FROM fleets WHERE fleet_id=?1 AND director_member_id=?2 AND deleted_at IS NULL)",
		[]any{run.fleetID, run.directorMemberID},
		func(row sqlRow) (bool, error) { return row.bool(0) })
	if err != nil {
		return runtimeEventView{}, err
	}
	if err := ensure(live, "permission fleet/Director changed"); err != nil {
		return runtimeEventView{}, err
	}
	return view, nil
}

// interactionMissingEvidence is the operation-centred facts an inspected interaction lacks; a
// question lacks none of them.
func interactionMissingEvidence(classification interactionClass, evidence jsontext.Value) ([]missingEvidence, error) {
	if classification == interactionClassQuestion {
		if err := validateEvidence(evidence); err != nil {
			return nil, err
		}
		return []missingEvidence{}, nil
	}
	return inspectionMissingEvidence(evidence)
}

func inspectionMissingEvidence(evidence jsontext.Value) ([]missingEvidence, error) {
	if err := validateEvidence(evidence); err != nil {
		return nil, err
	}
	return []missingEvidence{
		missingEvidenceOperation,
		missingEvidenceArguments,
		missingEvidenceCwd,
		missingEvidenceEnvironment,
		missingEvidenceTargets,
		missingEvidenceEffects,
		missingEvidenceChoiceScope,
	}, nil
}

func combineManualProgress[T any](progress T, progressErr, accounting error) (T, error) {
	switch {
	case progressErr == nil && accounting == nil:
		return progress, nil
	case progressErr != nil && accounting != nil:
		return progress, cafleetErrorOf(progressErr).withDiagnostic(cafleetErrorOf(accounting).message())
	case progressErr != nil:
		return progress, progressErr
	default:
		return progress, accounting
	}
}

// inputExpiration is an input deadline as an absolute monotonic-clock nanosecond reading, so it
// crosses a process boundary.
type inputExpiration struct {
	expiresAt uint64
}

func inputExpirationCapture(deadline time.Time, c clock) (inputExpiration, error) {
	sample, err := c.monotonicSample()
	if err != nil {
		return inputExpiration{}, err
	}
	if err := sample.validate(); err != nil {
		return inputExpiration{}, err
	}
	remaining := deadline.Sub(sample.after)
	if remaining <= 0 {
		return inputExpiration{}, deadlineExceededError()
	}
	remainingNs := uint64(remaining) //nolint:gosec // remaining is positive
	if remainingNs > math.MaxUint64-sample.monotonicNs {
		return inputExpiration{}, runtimeError("input expiration overflows monotonic nanoseconds")
	}
	return inputExpiration{expiresAt: sample.monotonicNs + remainingNs}, nil
}

func (e inputExpiration) receive(admittedAt time.Time, c clock) (time.Time, error) {
	sample, err := c.monotonicSample()
	if err != nil {
		return time.Time{}, err
	}
	if err := sample.validate(); err != nil {
		return time.Time{}, err
	}
	if e.expiresAt <= sample.monotonicNs {
		return time.Time{}, deadlineExceededError()
	}
	remainingNs := e.expiresAt - sample.monotonicNs
	if remainingNs > math.MaxInt64 {
		return time.Time{}, runtimeError("input expiration exceeds the local clock range")
	}
	expiration := sample.before.Add(time.Duration(remainingNs))
	admission := admittedAt.Add(5 * time.Second)
	deadline := expiration
	if admission.Before(deadline) {
		deadline = admission
	}
	if err := runtimeCheckDeadline(sample.after, deadline); err != nil {
		return time.Time{}, err
	}
	return deadline, nil
}

func (e inputExpiration) monotonicNs() uint64 {
	return e.expiresAt
}

func receiveInputExpiration(expiresAtMonoNs uint64, admittedAt time.Time, c clock) (time.Time, error) {
	return inputExpiration{expiresAt: expiresAtMonoNs}.receive(admittedAt, c)
}

// requestTokenSource fills a permission request token's random bytes.
type requestTokenSource interface {
	fill(bytes *[32]byte) error
}

const (
	receiptCacheCapacity               = 4096
	stageSettle                        = 500 * time.Millisecond
	stageWriteMargin                   = 250 * time.Millisecond
	deferredAttemptBudget              = 5 * time.Second
	deferredFinalization               = time.Second
	stallBound                         = 180 * time.Second
	runtimeInputPostSubmitVerification = 2 * time.Second
	pasteCloseReserve                  = 250 * time.Millisecond
	pasteOpen                          = "\x1b[200~"
	pasteClose                         = "\x1b[201~"
	fallbackEscape                     = 0x1b
	fallbackEnter                      = 0x0d
	answerAlreadySent                  = "an answer was already sent for this episode"
)

// inputBoundary names the coordinator step a test hook interrupts; production has no hooks.
type inputBoundary uint8

const (
	inputBoundaryReserved inputBoundary = iota + 1
	inputBoundaryBeforeWrite
	inputBoundaryAfterWrite
	inputBoundaryTerminalState
)

// attemptKey names a retained attempt: a durable prompt or exec receipt, or one of the two
// receipt-free attempts the runner holds only in memory. Its variants are comparable map keys.
//
//sumtype:decl
type attemptKey interface{ isAttemptKey() }

type attemptKeyReceipt struct{ id int64 }

type attemptKeySweepPreview struct{}

type attemptKeyAnswer struct{}

func (attemptKeyReceipt) isAttemptKey()      {}
func (attemptKeySweepPreview) isAttemptKey() {}
func (attemptKeyAnswer) isAttemptKey()       {}

func attemptKeyReceiptOf(key attemptKey) (int64, bool) {
	switch key := key.(type) {
	case attemptKeyReceipt:
		return key.id, true
	case attemptKeySweepPreview, attemptKeyAnswer:
		return 0, false
	default:
		panic("invalid attemptKey")
	}
}

func attemptKeyReceiptID(key attemptKey) int64 {
	id, ok := attemptKeyReceiptOf(key)
	if !ok {
		panic("only a durable receipt attempt reaches its stored row")
	}
	return id
}

// attemptRecord is the progress record of a retained attempt: the stored row of a receipt, or the
// in-memory state of a receipt-free attempt. Its variants are comparable, so == is Rust's
// PartialEq.
//
//sumtype:decl
type attemptRecord interface{ isAttemptRecord() }

type attemptRecordReceipt struct{ operation inputOperation }

type attemptRecordReceiptFree struct{ record receiptFreeRecord }

func (attemptRecordReceipt) isAttemptRecord()     {}
func (attemptRecordReceiptFree) isAttemptRecord() {}

type receiptFreeRecord struct {
	state          inputState
	deliveryMethod opt[deliveryMethod]
	deliveryReason opt[string]
	stopCause      opt[stopCause]
}

func attemptRecordState(record attemptRecord) inputState {
	switch record := record.(type) {
	case attemptRecordReceipt:
		return record.operation.state
	case attemptRecordReceiptFree:
		return record.record.state
	default:
		panic("invalid attemptRecord")
	}
}

func attemptRecordDeliveryMethod(record attemptRecord) opt[deliveryMethod] {
	switch record := record.(type) {
	case attemptRecordReceipt:
		return record.operation.deliveryMethod
	case attemptRecordReceiptFree:
		return record.record.deliveryMethod
	default:
		panic("invalid attemptRecord")
	}
}

func attemptRecordDeliveryReason(record attemptRecord) opt[string] {
	switch record := record.(type) {
	case attemptRecordReceipt:
		return record.operation.deliveryReason
	case attemptRecordReceiptFree:
		return record.record.deliveryReason
	default:
		panic("invalid attemptRecord")
	}
}

func attemptRecordStopCause(record attemptRecord) opt[stopCause] {
	switch record := record.(type) {
	case attemptRecordReceipt:
		return record.operation.stopCause
	case attemptRecordReceiptFree:
		return record.record.stopCause
	default:
		panic("invalid attemptRecord")
	}
}

func attemptRecordReceiptOf(record attemptRecord) inputOperation {
	switch record := record.(type) {
	case attemptRecordReceipt:
		return record.operation
	case attemptRecordReceiptFree:
		panic("a receipt-free attempt has no stored row")
	default:
		panic("invalid attemptRecord")
	}
}

type inputCheckpoint struct {
	attempt      attemptKey
	stage        submissionStage
	writtenBytes uint64
	outputSeq    uint64
}

// inputHooks lets a test act at a coordinator boundary; production services have none.
type inputHooks interface {
	checkpoint(boundary inputBoundary, state inputCheckpoint, deadline time.Time) error
}

// inputServices is the coordinator's collaborators; stageSettle is stageSettle in production and
// hooks nil, and tests override both.
type inputServices struct {
	catalog     *profileCatalog
	tokenSource requestTokenSource
	clock       clock
	stageSettle time.Duration
	hooks       inputHooks
}

func newInputServices(catalog *profileCatalog, tokenSource requestTokenSource, c clock) inputServices {
	return inputServices{catalog: catalog, tokenSource: tokenSource, clock: c, stageSettle: stageSettle}
}

// previewWait is the sweep's selected preview while the composer holds it; the stall watchdog
// runs from since.
type previewWait struct {
	messageID int64
	since     time.Time
	stalled   bool
}

// blockedWait is queued input or a pending preview waiting behind the run's lowest uncertain
// ordinary input; the stall watchdog runs from since.
type blockedWait struct {
	blockerInputID int64
	since          time.Time
	stalled        bool
}

// pendingFinalization is a deferred input's terminal zero-byte transaction that storage
// prevented; it holds only metadata, blocks every later ordinary input and retries on the
// deferred timer.
type pendingFinalization struct {
	inputID  int64
	expected inputState
	reason   string
}

// storageFailure reports a transient storage obstacle that a later tick may clear; every other
// failure propagates.
func storageFailure(err error) bool {
	kind := cafleetErrorOf(err).kind
	return kind == errorDatabaseBusy || kind == errorDeadlineExceeded
}

// attemptKind is what a runner attempt delivers; only prompt and exec attempts reserve a durable
// receipt.
//
//sumtype:decl
type attemptKind interface{ isAttemptKind() }

type attemptKindPreview struct{ messageID int64 }

type attemptKindPrompt struct{}

type attemptKindExec struct{}

type attemptKindAnswer struct{}

func (attemptKindPreview) isAttemptKind() {}
func (attemptKindPrompt) isAttemptKind()  {}
func (attemptKindExec) isAttemptKind()    {}
func (attemptKindAnswer) isAttemptKind()  {}

func attemptKindReceipt(kind inputKind) attemptKind {
	switch kind {
	case inputKindPrompt:
		return attemptKindPrompt{}
	case inputKindExec:
		return attemptKindExec{}
	default:
		panic("invalid inputKind")
	}
}

func attemptKindReceiptKind(kind attemptKind) (inputKind, bool) {
	switch kind.(type) {
	case attemptKindPrompt:
		return inputKindPrompt, true
	case attemptKindExec:
		return inputKindExec, true
	case attemptKindPreview, attemptKindAnswer:
		return 0, false
	default:
		panic("invalid attemptKind")
	}
}

// attemptRequest is an attempt's identity in runner memory.
type attemptRequest struct {
	operationKey  string
	kind          attemptKind
	payloadDigest string
}

func attemptRequestReceipt(input newInput) attemptRequest {
	return attemptRequest{operationKey: input.operationKey, kind: attemptKindReceipt(input.kind), payloadDigest: input.payloadDigest}
}

// newInput is the durable receipt request of a prompt or exec attempt.
func (r attemptRequest) newInput() newInput {
	kind, ok := attemptKindReceiptKind(r.kind)
	if !ok {
		panic("only prompt and exec attempts reserve a receipt")
	}
	return newInput{operationKey: r.operationKey, kind: kind, payloadDigest: r.payloadDigest}
}

// queuedInput is runner-owned ordinary input accepted with a durable ready receipt; its literal
// stays in memory only. headSince is when this entry last became a head that no interaction
// blocks; the stall watchdog runs from there, and an attempt start or a blocking interaction
// clears it.
type queuedInput struct {
	inputID         int64
	stalled         bool
	request         newInput
	literal         string
	operation       composerOperation
	attemptDeadline opt[time.Time]
	obstacle        string
	headSince       opt[time.Time]
}

type deliverySelection struct {
	method deliveryMethod
	reason opt[string]
	stages [][]byte
}

//sumtype:decl
type deliveryChoice interface{ isDeliveryChoice() }

type deliveryChoiceReady struct{ selection deliverySelection }

type deliveryChoiceWaiting struct{}

type deliveryChoicePlainLimit struct{ cause string }

func (deliveryChoiceReady) isDeliveryChoice()      {}
func (deliveryChoiceWaiting) isDeliveryChoice()    {}
func (deliveryChoicePlainLimit) isDeliveryChoice() {}

type composerInput struct {
	operation composerOperation
	kind      attemptKind
	literal   string
}

type deferredAttempt struct {
	queued   queuedInput
	obstacle string
}

type currentRequest struct {
	observed          observedRequest
	interactionID     int64
	eventID           int64
	messageID         int64
	manualGeneration  uint64
	pointerGeneration uint64
}

func (r *currentRequest) sameRequest(other *currentRequest) bool {
	return r.observed.requestGeneration == other.observed.requestGeneration && r.interactionID == other.interactionID &&
		r.eventID == other.eventID && r.messageID == other.messageID && r.manualGeneration == other.manualGeneration &&
		r.pointerGeneration == other.pointerGeneration && r.observed.sameRequest(&other.observed)
}

// tokenedRequest is the current request with the token minted for it.
type tokenedRequest struct {
	request currentRequest
	token   string
}

func answeredClassification(key answerKey) interactionClass {
	switch key.(type) {
	case answerKeyDecision:
		return interactionClassPermissionRequest
	case answerKeyChoice:
		return interactionClassQuestion
	default:
		panic("invalid answerKey")
	}
}

func answerKindMismatch(classification interactionClass, key answerKey) opt[string] {
	question := classification == interactionClassQuestion
	switch key.(type) {
	case answerKeyDecision:
		if question {
			return some("decision answers apply only to permission requests")
		}
	case answerKeyChoice:
		if !question {
			return some("choice answers apply only to question interactions")
		}
	default:
		panic("invalid answerKey")
	}
	return none[string]()
}

type inputPhase uint8

const (
	inputPhaseAwaitReservation inputPhase = iota + 1
	inputPhaseAwaitBeforeWrite
	inputPhaseArmed
	inputPhaseBlocked
	inputPhaseClosing
	inputPhaseVerifying
)

// permissionSelection is an answer's request, binding and plan; it is shared by pointer, so
// Rust's Arc::ptr_eq is pointer equality.
type permissionSelection struct {
	request      permissionRespondInput
	binding      currentRequest
	continuation *observedContinuation
	gapCount     uint64
	plan         answerPlan
}

type stageGateProgress uint8

const (
	stageGateProgressReady stageGateProgress = iota + 1
	stageGateProgressWaiting
)

// gateFinalizationReserve fails a gate still unmet this close to the response deadline, so the
// written stages' uncertainty is persisted within the original budget.
const gateFinalizationReserve = 100 * time.Millisecond

//sumtype:decl
type retainedSelection interface{ isRetainedSelection() }

type retainedSelectionComposer struct {
	method    deliveryMethod
	stages    [][]byte
	nextStage int
	offset    int
	closing   bool
}

type retainedSelectionPermission struct {
	selection *permissionSelection
	nextStage int
}

func (*retainedSelectionComposer) isRetainedSelection()   {}
func (*retainedSelectionPermission) isRetainedSelection() {}

//sumtype:decl
type pendingDelivery interface{ isPendingDelivery() }

type pendingDeliveryPermission struct{ selection *permissionSelection }

type pendingDeliveryComposer struct{ composer *composerDelivery }

func (pendingDeliveryPermission) isPendingDelivery() {}
func (pendingDeliveryComposer) isPendingDelivery()   {}

type pendingInput struct {
	reservation opt[attemptKey]
	request     attemptRequest
	bytes       []byte
	delivery    pendingDelivery
	deadline    time.Time
	phase       inputPhase
}

type composerDelivery struct {
	method        deliveryMethod
	reason        opt[string]
	stages        [][]byte
	writtenStages int
	started       bool
	settleUntil   opt[time.Time]
	stop          opt[composerStop]
	stopFailure   *cafleetError
	verification  *postSubmitVerification
	// pane is the pane-input lock this delivery holds, nil when it holds none; releasePane
	// releases it.
	pane      *paneLock
	literal   string
	operation composerOperation
	deferred  *deferredAttempt
}

type composerStop struct {
	cause          stopCause
	completePrefix bool
}

type postSubmitVerification struct {
	until      time.Time
	outputSeq  uint64
	episodeSeq opt[int64]
}

func (c *composerDelivery) requiresEmptyComposer() bool {
	return c.method == deliveryMethodProfile && !c.started
}

// releasePane releases the delivery's pane-input lock, as Rust drops its PaneLock.
func (c *composerDelivery) releasePane() {
	if c.pane != nil {
		c.pane.close()
		c.pane = nil
	}
}

// releasePane releases a composer delivery's pane-input lock; a permission answer holds none.
func (p *pendingInput) releasePane() {
	if composer := p.composer(); composer != nil {
		composer.releasePane()
	}
}

type ordinaryBlocker uint8

const (
	ordinaryBlockerInteraction ordinaryBlocker = iota + 1
	ordinaryBlockerObserverUnavailable
)

func ordinaryBlockerOf(status observationStatus) opt[ordinaryBlocker] {
	hold, held := status.eligibility.(inputEligibilityHold)
	if status.visibility == visibilityVisible || status.visibility == visibilityObscured ||
		(held && (hold.reason == gateReasonCandidate || hold.reason == gateReasonInteraction)) || status.retainedTransition {
		return some(ordinaryBlockerInteraction)
	}
	_, diagnosed := status.diagnostic.get()
	if (held && (hold.reason == gateReasonObserverUnavailable || hold.reason == gateReasonGap)) || diagnosed {
		return some(ordinaryBlockerObserverUnavailable)
	}
	return none[ordinaryBlocker]()
}

func interactionCause(status observationStatus) stopCause {
	if status.eligibility == inputEligibility(inputEligibilityHold{reason: gateReasonCandidate}) {
		return stopCauseInteractionCandidate
	}
	return stopCauseInteractionOpen
}

// stalledRecord is an input_delivery_stalled record for exactly one waiting head or preview whose
// wait began at since.
func stalledRecord(inputID, messageID opt[int64], since, now time.Time, c clock) jsontext.Value {
	return jsonValue(jsonObject{
		{"kind", "input_delivery_stalled"},
		{"input_id", inputID},
		{"message_id", messageID},
		{"blocked_by_input_id", nil},
		{"waiting_since", waitingSince(since, now, c)},
	})
}

// blockedRecord is an input_delivery_stalled record for queued input or a pending preview held
// behind the uncertain blocker since since.
func blockedRecord(blocker int64, since, now time.Time, c clock) jsontext.Value {
	return jsonValue(jsonObject{
		{"kind", "input_delivery_stalled"},
		{"input_id", nil},
		{"message_id", nil},
		{"blocked_by_input_id", blocker},
		{"waiting_since", waitingSince(since, now, c)},
	})
}

func waitingSince(since, now time.Time, c clock) string {
	waited := now.Sub(since)
	if waited < 0 {
		panic("bounded stall wait")
	}
	return formatUTC(c.wallNow().Add(-waited))
}

// receiptFreeRecordOf is the in-progress in-memory record of a receipt-free attempt with its
// selected delivery.
func receiptFreeRecordOf(selection opt[reservationSelection]) attemptRecord {
	record := receiptFreeRecord{state: inputStateInProgress, deliveryMethod: none[deliveryMethod](), deliveryReason: none[string](), stopCause: none[stopCause]()}
	if selected, set := selection.get(); set {
		record.deliveryMethod = some(selected.method)
		record.deliveryReason = selected.reason
	}
	return attemptRecordReceiptFree{record: record}
}

func fallbackStages(prefix []byte, literal string) ([][]byte, error) {
	if len(prefix)+len(literal)+1 > 65536 {
		return nil, runtimeError("native composer payload exceeds 65536 bytes")
	}
	text := make([]byte, 0, len(prefix)+len(literal))
	text = append(append(text, prefix...), literal...)
	return [][]byte{{fallbackEscape}, text, {fallbackEnter}}, nil
}

// attempt is the installed receipt or in-memory record this attempt targets.
func (p *pendingInput) attempt() opt[attemptKey] {
	return p.reservation
}

// receiptID is the durable receipt this attempt targets, absent for a receipt-free attempt.
func (p *pendingInput) receiptID() opt[int64] {
	key, set := p.reservation.get()
	if !set {
		return none[int64]()
	}
	if id, receipt := attemptKeyReceiptOf(key); receipt {
		return some(id)
	}
	return none[int64]()
}

func (p *pendingInput) permission() *permissionSelection {
	switch delivery := p.delivery.(type) {
	case pendingDeliveryPermission:
		return delivery.selection
	case pendingDeliveryComposer:
		return nil
	default:
		panic("invalid pendingDelivery")
	}
}

func (p *pendingInput) composer() *composerDelivery {
	switch delivery := p.delivery.(type) {
	case pendingDeliveryComposer:
		return delivery.composer
	case pendingDeliveryPermission:
		return nil
	default:
		panic("invalid pendingDelivery")
	}
}

func (p *pendingInput) deferred() *deferredAttempt {
	if composer := p.composer(); composer != nil {
		return composer.deferred
	}
	return nil
}

type retainedInput struct {
	operation        attemptRecord
	bytes            []byte
	selection        retainedSelection
	stage            submissionStage
	writtenBytes     uint64
	outputSeq        uint64
	attemptAvailable bool
	leased           bool
}

func selectedAnswerStage(plan *answerPlan, index int) (*answerStage, error) {
	if index < 0 || index >= len(plan.stages) {
		return nil, runtimeError("selected answer stage is absent")
	}
	return &plan.stages[index], nil
}

type guardPermit uint8

const (
	guardPermitFresh guardPermit = iota + 1
	guardPermitRejected
	guardPermitConsumedPreIo
	guardPermitIoPossible
	guardPermitResultRecorded
)

// inputGuard is the write lease of one retained attempt. While it lives the coordinator does not
// touch the attempt; close, the Rust Drop, ends the lease and is idempotent.
type inputGuard struct {
	retained *retainedInput
	proof    *drainedInput
	deadline time.Time
	permit   guardPermit
	clock    clock
}

func (g *inputGuard) beginWrite(t *transport, bytes []byte, deadline time.Time) (time.Time, error) {
	retained := g.retained
	if retained == nil {
		return time.Time{}, runtimeError("native writer is absent")
	}
	if err := ensure(g.permit == guardPermitFresh, "input write permit is consumed"); err != nil {
		return time.Time{}, err
	}
	g.permit = guardPermitRejected
	if deadline.Before(g.deadline) {
		g.deadline = deadline
	}
	if err := runtimeCheckDeadline(g.clock.now(), g.deadline); err != nil {
		return time.Time{}, err
	}
	if err := ensure(string(bytes) == string(retained.bytes), "input bytes do not match the selected stage"); err != nil {
		return time.Time{}, err
	}
	if err := ensure(attemptRecordState(retained.operation) == inputStateInProgress && retained.leased, "input write requires its retained reservation"); err != nil {
		return time.Time{}, err
	}
	switch selection := retained.selection.(type) {
	case *retainedSelectionComposer:
		if selection.closing {
			if err := ensure(string(bytes) == pasteClose, "paste cleanup writes only its closing marker"); err != nil {
				return time.Time{}, err
			}
			g.permit = guardPermitConsumedPreIo
			return g.deadline, nil
		}
		if g.proof == nil {
			return time.Time{}, runtimeError("native drain is absent")
		}
		selected := selection.nextStage < len(selection.stages) && selection.offset <= len(selection.stages[selection.nextStage]) &&
			string(selection.stages[selection.nextStage][selection.offset:]) == string(bytes)
		if err := ensure(selected, "composer stage selection changed"); err != nil {
			return time.Time{}, err
		}
		var err error
		if selection.method == deliveryMethodProfile && selection.nextStage == 0 && selection.offset == 0 {
			err = t.validateInputDrain(g.proof, g.deadline)
		} else {
			err = t.validateObservationDrain(g.proof, g.deadline)
		}
		if err != nil {
			return time.Time{}, err
		}
	case *retainedSelectionPermission:
		stage, err := selectedAnswerStage(&selection.selection.plan, selection.nextStage)
		if err != nil {
			return time.Time{}, err
		}
		if err := ensure(string(stage.bytes) == string(bytes), "permission stage selection changed"); err != nil {
			return time.Time{}, err
		}
		if g.proof == nil {
			return time.Time{}, runtimeError("native drain is absent")
		}
		if err := t.validatePermission(g.proof, g.deadline); err != nil {
			return time.Time{}, err
		}
	default:
		panic("invalid retainedSelection")
	}
	g.permit = guardPermitConsumedPreIo
	return g.deadline, nil
}

func (g *inputGuard) markIoPossible() {
	if g.permit != guardPermitConsumedPreIo {
		panic("possible I/O requires a validated consumed permit")
	}
	g.permit = guardPermitIoPossible
}

func (g *inputGuard) recordWriteResult(count int, writeErr error, offered int) error {
	retained := g.retained
	if retained == nil {
		return runtimeError("native writer is absent")
	}
	if err := ensure(g.permit == guardPermitIoPossible, "input result requires its possible-I/O attempt"); err != nil {
		return err
	}
	if err := ensure(offered == len(retained.bytes), "input offered count does not match its selected stage"); err != nil {
		return err
	}
	switch {
	case writeErr == nil:
		if err := ensure(count >= 0 && count <= offered, "native input count exceeds its offer"); err != nil {
			return err
		}
		if uint64(count) > math.MaxUint64-retained.writtenBytes { //nolint:gosec // count is checked nonnegative above
			return runtimeError("native input count exhausted")
		}
		retained.writtenBytes += uint64(count) //nolint:gosec // count is checked nonnegative above
		if count == offered {
			switch selection := retained.selection.(type) {
			case *retainedSelectionComposer:
				if selection.closing {
					retained.stage = submissionStagePartial
					g.permit = guardPermitResultRecorded
					return nil
				}
				selection.offset = 0
				selection.nextStage++
				if selection.nextStage < len(selection.stages) {
					retained.bytes = selection.stages[selection.nextStage]
					retained.stage = submissionStagePartial
				} else {
					retained.stage = submissionStageSubmitted
				}
			case *retainedSelectionPermission:
				if _, err := selectedAnswerStage(&selection.selection.plan, selection.nextStage); err != nil {
					return err
				}
				selection.nextStage++
				if selection.nextStage == len(selection.selection.plan.stages) {
					retained.stage = submissionStageSubmitted
				} else {
					stage, err := selectedAnswerStage(&selection.selection.plan, selection.nextStage)
					if err != nil {
						return err
					}
					retained.bytes = stage.bytes
					retained.stage = submissionStagePartial
				}
			default:
				panic("invalid retainedSelection")
			}
		} else if count > 0 {
			retained.stage = submissionStagePartial
			if selection, composer := retained.selection.(*retainedSelectionComposer); composer && !selection.closing && selection.nextStage+1 < len(selection.stages) {
				selection.offset += count
				retained.bytes = selection.stages[selection.nextStage][selection.offset:]
			}
		}
	case wouldBlockOrInterrupted(writeErr):
	default:
		if retained.stage == submissionStageZeroBytes {
			retained.stage = submissionStageUnknown
		}
	}
	g.permit = guardPermitResultRecorded
	return nil
}

// close ends the lease: after possible I/O a zero-byte stage becomes unknown.
func (g *inputGuard) close() {
	retained := g.retained
	if retained == nil {
		return
	}
	g.retained = nil
	if g.permit == guardPermitIoPossible && retained.stage == submissionStageZeroBytes {
		retained.stage = submissionStageUnknown
	}
	retained.leased = false
}

type drainRequirement uint8

const (
	drainRequirementObservation drainRequirement = iota + 1
	drainRequirementEmptyComposer
	drainRequirementNoBlockingInteraction
)

type writeValidation uint8

const (
	writeValidationReady writeValidation = iota + 1
	writeValidationContended
)

// inputCoordinator owns programmatic input to one run's native child. One runner goroutine owns
// it; every field that releases a resource when cleared (pending) changes only through
// setPending, clearPending and takePending.
type inputCoordinator struct {
	run                    runRecord
	backend                codingAgent
	lock                   *runLock
	services               inputServices
	manualGeneration       uint64
	pointerGeneration      uint64
	manualAccountingFailed bool
	placement              placement
	operations             map[attemptKey]*retainedInput
	// answeredInteractions holds interactions whose episode has had an answer byte written; each
	// accepts no further answer.
	answeredInteractions map[int64]struct{}
	pending              *pendingInput
	currentRequest       *tokenedRequest
	inputFenced          bool
	observationBlocker   opt[string]
	databaseURL          string
	queue                []queuedInput
	finalization         *pendingFinalization
	deferredFailed       bool
	outputSeq            uint64
	bracketedPaste       bool
	stallBound           time.Duration
	previewWait          *previewWait
	blockedWait          *blockedWait
	diagnostics          runnerDiagnostics
	// composerReader reads the composer text back for the pre-submit comparison.
	composerReader *detector
}

// setPending installs the coordinator's one programmatic attempt.
func (c *inputCoordinator) setPending(pending *pendingInput) {
	if c.pending != nil {
		panic("one programmatic attempt is pending at a time")
	}
	c.pending = pending
}

// clearPending drops the pending attempt and releases its pane-input lock, as Rust's
// `self.pending = None` drops its PaneLock.
func (c *inputCoordinator) clearPending() {
	if c.pending != nil {
		c.pending.releasePane()
	}
	c.pending = nil
}

// takePending detaches the pending attempt; its caller releases the pane-input lock it holds
// when it is done with it, as Rust drops the taken value at the end of its scope.
func (c *inputCoordinator) takePending() *pendingInput {
	pending := c.pending
	if pending == nil {
		panic("a pending attempt is taken only when present")
	}
	c.pending = nil
	return pending
}

func (c *inputCoordinator) stageSettle() time.Duration {
	return c.services.stageSettle
}

func (c *inputCoordinator) now() time.Time {
	return c.services.clock.now()
}

func (c *inputCoordinator) inspectReceived(ctx context.Context, conn *dbConn, input permissionInspectInput, observation *observationController, t *transport, deadline time.Time) (inputControlReply, *inputFailure) {
	reply, err := c.inspect(ctx, conn, input, observation, t, deadline)
	if err != nil {
		c.currentRequest = nil
		return nil, c.runFailure(cafleetErrorOf(err), some(input.InteractionID), none[int64](), none[string]())
	}
	return reply, nil
}

func (c *inputCoordinator) inspect(ctx context.Context, conn *dbConn, input permissionInspectInput, observation *observationController, t *transport, deadline time.Time) (inputControlReply, error) {
	deadline = boundedByInherited(ctx, deadline)
	if err := runtimeCheckDeadline(c.now(), deadline); err != nil {
		return nil, err
	}
	view, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (runtimeEventView, error) {
		return inTransaction(conn, transactionDeferred, func() (runtimeEventView, error) {
			if err := c.validateLiveRun(ctx, conn); err != nil {
				return runtimeEventView{}, err
			}
			view, err := inspectionAssociation(ctx, conn, c.run, input)
			if err != nil {
				return runtimeEventView{}, err
			}
			return view, runtimeCheckDeadline(c.now(), deadline)
		})
	})
	if err != nil {
		return nil, err
	}
	interaction, set := view.interaction.get()
	if !set {
		panic("validated current permission interaction")
	}
	messageID, set := view.event.messageID.get()
	if !set {
		panic("validated inbox message")
	}
	evidence := observation.currentEvidence()
	if evidence == nil {
		return nil, runtimeError("current inspection observation unavailable")
	}
	missing, err := interactionMissingEvidence(interaction.classification, evidence)
	if err != nil {
		return nil, err
	}
	fresh := false
	if proof := t.takeInputDrain(); proof != nil {
		if err := t.validatePermission(proof, deadline); err != nil {
			return nil, err
		}
		fresh = true
	}
	observed := observation.currentRequest
	if current := c.currentRequest; current != nil &&
		(current.request.observed.requestGeneration != observation.requestGeneration ||
			current.request.manualGeneration != c.manualGeneration ||
			current.request.pointerGeneration != c.pointerGeneration || observed == nil) {
		c.currentRequest = nil
	}
	supportedDecisions := []permissionDecision{}
	supportedChoices := []supportedChoice{}
	answerRuleID := none[string]()
	requestToken := none[string]()
	blocker := c.permissionBlocker(observation)
	reason := some("current executable permission request unavailable")
	if text, blocked := blocker.get(); blocked {
		reason = some(text)
	}
	_, blocked := blocker.get()
	switch {
	case fresh && !blocked && observed != nil:
		if err := ensure(observed.episodeSeq == interaction.episodeSeq && observed.classification == interaction.classification, "permission inspection durable episode changed"); err != nil {
			return nil, err
		}
		next := currentRequest{
			observed:          *observed,
			interactionID:     interaction.interactionID,
			eventID:           view.event.eventID,
			messageID:         messageID,
			manualGeneration:  c.manualGeneration,
			pointerGeneration: c.pointerGeneration,
		}
		if len(next.observed.answerMatches) == 0 {
			break
		}
		if c.currentRequest != nil && c.currentRequest.request.sameRequest(&next) {
			c.currentRequest.request.observed.outputSeq = next.observed.outputSeq
		} else {
			c.currentRequest = nil
			var bytes [32]byte
			if err := c.services.tokenSource.fill(&bytes); err != nil {
				return nil, err
			}
			c.currentRequest = &tokenedRequest{request: next, token: digestHex(bytes[:])}
		}
		current := c.currentRequest
		first := current.request.observed.answerMatches[0].plan.capability
		if !slices.ContainsFunc(current.request.observed.answerMatches, func(m answerMatch) bool {
			return m.plan.capability.answerRuleID != first.answerRuleID
		}) {
			answerRuleID = some(first.answerRuleID)
		}
		for _, matched := range current.request.observed.answerMatches {
			switch key := matched.plan.capability.key.(type) {
			case answerKeyDecision:
				supportedDecisions = append(supportedDecisions, key.decision)
			case answerKeyChoice:
				supportedChoices = append(supportedChoices, supportedChoice{ChoiceID: key.choice, Label: matched.choiceLabel, Scope: matched.plan.choiceScope})
			default:
				panic("invalid answerKey")
			}
		}
		requestToken = some(current.token)
		reason = none[string]()
	case fresh && !blocked && observation.unplannedQuestionEpisode == some(interaction.episodeSeq):
		reason = some("question has no tested answer")
	}
	if err := runtimeCheckDeadline(c.now(), deadline); err != nil {
		return nil, err
	}
	capability := coverageUnknown
	if _, tokened := requestToken.get(); tokened {
		capability = coverageTested
	}
	return permissionInspection{
		EventID:            view.event.eventID,
		MessageID:          messageID,
		MemberID:           c.run.memberID,
		FleetID:            c.run.fleetID,
		DirectorMemberID:   c.run.directorMemberID,
		RunID:              c.run.runID,
		OwnerEpoch:         c.run.ownerEpoch,
		InteractionID:      interaction.interactionID,
		EpisodeSeq:         interaction.episodeSeq,
		Classification:     observation.currentClassification,
		Visibility:         observation.status().visibility,
		Backend:            c.run.backend,
		NativeVersion:      c.run.nativeVersion,
		AnswerRuleID:       answerRuleID,
		AnswerCapability:   capability,
		CapabilityReason:   reason,
		SupportedDecisions: supportedDecisions,
		SupportedChoices:   supportedChoices,
		RequestToken:       requestToken,
		OutputSeq:          t.snapshot().screenSeq,
		Evidence:           evidence,
		MissingEvidence:    missing,
	}, nil
}

// fence cancels queued work as runner_stopped and returns the settled input IDs, whose results
// the caller publishes before the run's exit event.
func (c *inputCoordinator) fence(ctx context.Context, deadline time.Time) ([]int64, error) {
	c.currentRequest = nil
	c.inputFenced = true
	settled := []int64{}
	if id, _, written, active := c.activeDeferred(); active {
		c.clearPending()
		conn, err := connect(c.databaseURL)
		if err != nil {
			return nil, err
		}
		if err := mergeCleanup(c.fenceDeferred(ctx, conn, id, written, deadline), conn.close()); err != nil {
			return nil, err
		}
		settled = append(settled, id)
	}
	if len(c.queue) > 0 || c.finalization != nil {
		conn, err := connect(c.databaseURL)
		if err != nil {
			return nil, err
		}
		ids, fenced := c.fenceQueue(ctx, conn, deadline)
		settled = append(settled, ids...)
		if err := mergeCleanup(fenced, conn.close()); err != nil {
			return nil, err
		}
	}
	if err := runtimeCheckDeadline(c.now(), deadline); err != nil {
		return nil, err
	}
	return settled, nil
}

func (c *inputCoordinator) fenceDeferred(ctx context.Context, conn *dbConn, id int64, written bool, deadline time.Time) error {
	if written {
		_, err := c.recordDeferredUncertain(ctx, conn, id, stopCauseRunnerStopped, deadline)
		return err
	}
	operation, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (inputOperation, error) {
		return settleZeroByteDeferred(ctx, conn, c.run.owner(), id, inputStateInProgress, "runner_stopped")
	})
	if err != nil {
		return err
	}
	retained := c.retained(attemptKeyReceipt{id: id}, "a deferred attempt retains its receipt")
	retained.operation = attemptRecordReceipt{operation: operation}
	retained.attemptAvailable = false
	return nil
}

func (c *inputCoordinator) fenceQueue(ctx context.Context, conn *dbConn, deadline time.Time) ([]int64, error) {
	settled := []int64{}
	owner := c.run.owner()
	if pending := c.finalization; pending != nil {
		c.finalization = nil
		if _, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (inputOperation, error) {
			return settleZeroByteDeferred(ctx, conn, owner, pending.inputID, pending.expected, pending.reason)
		}); err != nil {
			return settled, err
		}
		settled = append(settled, pending.inputID)
	}
	for len(c.queue) > 0 {
		queued := c.queue[0]
		c.queue = c.queue[1:]
		if _, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (inputOperation, error) {
			return settleZeroByteDeferred(ctx, conn, owner, queued.inputID, inputStateReady, "runner_stopped")
		}); err != nil {
			return settled, err
		}
		settled = append(settled, queued.inputID)
	}
	return settled, nil
}

// retained is the retained attempt key names, which the caller's invariant guarantees.
func (c *inputCoordinator) retained(key attemptKey, invariant string) *retainedInput {
	retained, ok := c.operations[key]
	if !ok {
		panic(invariant)
	}
	return retained
}

func (c *inputCoordinator) permissionBlocker(observation *observationController) opt[string] {
	status := observation.status()
	code, diagnosed := status.diagnostic.get()
	switch {
	case c.inputFenced:
		return some("permission input is fenced")
	case c.manualAccountingFailed:
		return some("manual input accounting failed")
	case c.observationBlocker.set:
		return c.observationBlocker
	case diagnosed:
		return some("observation degraded: " + code)
	case status.retainedDraft:
		return some("observation update pending")
	case status.visibility != visibilityVisible:
		return some("permission request is not visible")
	case status.eligibility != inputEligibility(inputEligibilityHold{reason: gateReasonInteraction}):
		return some("permission request is not the current interaction")
	default:
		return none[string]()
	}
}

func (c *inputCoordinator) validatePermissionBinding(selection *permissionSelection, observation *observationController) error {
	if reason, blocked := c.permissionBlocker(observation).get(); blocked {
		return runtimeError("permission request is blocked: " + reason)
	}
	current := c.currentRequest
	if current == nil {
		return runtimeError("current permission token unavailable")
	}
	bound := &selection.binding
	if continuation := selection.continuation; continuation != nil {
		live := observation.currentContinuation
		return ensure(current.token == selection.request.RequestToken && c.manualGeneration == bound.manualGeneration &&
			c.pointerGeneration == bound.pointerGeneration && observation.gapCount == selection.gapCount &&
			observation.requestGeneration == continuation.requestGeneration && live != nil && live.equal(continuation),
			"permission continuation binding changed")
	}
	observed := observation.currentRequest
	if observed == nil {
		return runtimeError("current permission request unavailable")
	}
	if err := ensure(current.token == selection.request.RequestToken && current.request.sameRequest(bound) &&
		c.manualGeneration == bound.manualGeneration && c.pointerGeneration == bound.pointerGeneration &&
		observation.requestGeneration == bound.observed.requestGeneration &&
		observed.requestGeneration == bound.observed.requestGeneration &&
		observed.classification == answeredClassification(selection.request.Key) && observed.sameRequest(&bound.observed),
		"permission request token or current plan binding changed"); err != nil {
		return err
	}
	return ensure(slices.ContainsFunc(bound.observed.answerMatches, func(m answerMatch) bool { return m.plan.equal(&selection.plan) }), "selected permission plan changed")
}

func findPlanMatch(matches []answerMatch, plan *answerPlan) *answerMatch {
	for index := range matches {
		if matches[index].plan.equal(plan) {
			return &matches[index]
		}
	}
	return nil
}

func matchFocusedChoice(m *answerMatch) opt[string] {
	if m == nil {
		return none[string]()
	}
	return m.focusedChoice
}

// advanceStageGate holds the plan before each gated stage until the observation shows that
// stage's gate. After a complete earlier stage it accepts exactly the transition the gate names
// and rebinds the in-flight selection to it without minting a token.
func (c *inputCoordinator) advanceStageGate(observation *observationController) (stageGateProgress, error) {
	pending := c.pending
	if pending == nil {
		return stageGateProgressReady, nil
	}
	selection := pending.permission()
	if selection == nil {
		return stageGateProgressReady, nil
	}
	key, reserved := pending.attempt().get()
	if !reserved {
		return 0, runtimeError("permission reservation absent")
	}
	retained, ok := c.operations[key]
	if !ok {
		return 0, runtimeError("retained permission selection absent")
	}
	permission, isPermission := retained.selection.(*retainedSelectionPermission)
	if !isPermission {
		return 0, runtimeError("permission stage target changed")
	}
	nextStage := permission.nextStage
	stage, err := selectedAnswerStage(&selection.plan, nextStage)
	if err != nil {
		return 0, err
	}
	if stage.gate == nil {
		return stageGateProgressReady, nil
	}
	bound := &selection.binding
	boundMatch := findPlanMatch(bound.observed.answerMatches, &selection.plan)
	var holds bool
	switch gate := stage.gate.(type) {
	case stageGateFocus:
		holds = selection.continuation == nil && matchFocusedChoice(boundMatch) == some(gate.choice)
	case stageGateScreen:
		holds = selection.continuation != nil && selection.continuation.ruleID == gate.rule
	default:
		panic("invalid stageGate")
	}
	if holds {
		return stageGateProgressReady, nil
	}
	if err := ensure(nextStage > 0, "the first stage's gate does not hold"); err != nil {
		return 0, err
	}
	_, observerDiagnosed := observation.status().diagnostic.get()
	if err := ensure(c.manualGeneration == bound.manualGeneration && c.pointerGeneration == bound.pointerGeneration &&
		!c.manualAccountingFailed && !c.inputFenced && !c.observationBlocker.set && !observerDiagnosed &&
		observation.gapCount == selection.gapCount, "input or observation changed between answer stages"); err != nil {
		return 0, err
	}
	if err := ensure(observation.activeEpisodeSeq() == some(bound.observed.episodeSeq), "the permission episode changed between answer stages"); err != nil {
		return 0, err
	}
	deadline := pending.deadline
	waiting := func() (stageGateProgress, error) {
		if err := ensure(c.now().Add(gateFinalizationReserve).Before(deadline), "the next stage's gate did not hold before the response deadline"); err != nil {
			return 0, err
		}
		return stageGateProgressWaiting, nil
	}
	observed := observation.currentRequest
	unchanged := observed != nil && selection.continuation == nil &&
		observed.requestGeneration == bound.observed.requestGeneration && observed.focus == bound.observed.focus
	var rebound *permissionSelection
	switch gate := stage.gate.(type) {
	case stageGateFocus:
		if err := ensure(observation.currentContinuation == nil, "an unexpected continuation screen appeared between answer stages"); err != nil {
			return 0, err
		}
		if observed == nil || unchanged {
			return waiting()
		}
		observedMatch := findPlanMatch(observed.answerMatches, &selection.plan)
		sameCapture := (observedMatch == nil) == (boundMatch == nil) &&
			(observedMatch == nil || observedMatch.focusCapture == boundMatch.focusCapture)
		if err := ensure(observed.episodeSeq == bound.observed.episodeSeq && observed.requestRuleID == bound.observed.requestRuleID &&
			observed.semantic.equal(&bound.observed.semantic) && slices.Equal(observed.choices, bound.observed.choices) &&
			sameCapture && matchFocusedChoice(observedMatch) == some(gate.choice),
			"the request changed other than to the next stage's focus"); err != nil {
			return 0, err
		}
		binding := *bound
		binding.observed.requestGeneration = observed.requestGeneration
		binding.observed.focus = observed.focus
		binding.observed.outputSeq = observed.outputSeq
		binding.observed.answerMatches = slices.Clone(observed.answerMatches)
		rebound = &permissionSelection{request: selection.request, binding: binding, gapCount: selection.gapCount, plan: selection.plan}
	case stageGateScreen:
		continuation := observation.currentContinuation
		if continuation == nil {
			if err := ensure(observed == nil || unchanged, "the request changed instead of opening its continuation"); err != nil {
				return 0, err
			}
			return waiting()
		}
		if err := ensure(continuation.ruleID == gate.rule && continuation.episodeSeq == bound.observed.episodeSeq, "a different continuation screen appeared"); err != nil {
			return 0, err
		}
		copied := *continuation
		rebound = &permissionSelection{request: selection.request, binding: *bound, continuation: &copied, gapCount: selection.gapCount, plan: selection.plan}
	default:
		panic("invalid stageGate")
	}
	if rebound.continuation == nil {
		if c.currentRequest == nil {
			return 0, runtimeError("current permission token unavailable")
		}
		c.currentRequest = &tokenedRequest{request: rebound.binding, token: c.currentRequest.token}
	}
	permission.selection = rebound
	pending.delivery = pendingDeliveryPermission{selection: rebound}
	return stageGateProgressReady, nil
}

func (c *inputCoordinator) validatePermissionAssociation(ctx context.Context, conn *dbConn, selection *permissionSelection) error {
	request := selection.request
	view, err := inspectionAssociation(ctx, conn, c.run, request.inspection())
	if err != nil {
		return err
	}
	interaction, set := view.interaction.get()
	if !set {
		panic("validated permission interaction")
	}
	if err := ensure(interaction.episodeSeq == selection.binding.observed.episodeSeq &&
		view.event.messageID == some(selection.binding.messageID) && request.ExpectedOwnerEpoch == c.run.ownerEpoch,
		"permission response current association changed"); err != nil {
		return err
	}
	capability := selection.plan.capability
	return ensure(capability.backend.name() == c.run.backend &&
		jsonIsString(jsonIndex(jsonIndex(view.event.payloadJSON, "evidence"), "rule_id"), capability.requestRuleID) &&
		capability.key == request.Key, "answer capability does not match this request")
}

func (c *inputCoordinator) validateRespondIdentity(request inputControlRequest, input permissionRespondInput) error {
	if err := validateRespond(input); err != nil {
		return err
	}
	expected := controlIdentityFromRun(c.run)
	expected.OwnerEpoch = input.ExpectedOwnerEpoch
	return ensure(request.Identity.equal(expected) && input.ExpectedRunID == c.run.runID, "permission response fixed identity mismatch")
}

func (c *inputCoordinator) answeredRefusal(input permissionRespondInput) opt[inputControlReply] {
	if _, answered := c.answeredInteractions[input.InteractionID]; answered {
		return some[inputControlReply](c.rejectedResponse(input, answerAlreadySent))
	}
	return none[inputControlReply]()
}

// answeredEpisodeRefusal refuses, with zero bytes, an answer for an episode that already has an
// answer byte written; nil when the episode is still answerable.
func (c *inputCoordinator) answeredEpisodeRefusal(request inputControlRequest) (opt[inputControlReply], *inputFailure) {
	operation, ok := request.Operation.(permissionRespondOperation)
	if !ok {
		panic("the answered-episode check receives a response operation")
	}
	if err := c.validateRespondIdentity(request, operation.input); err != nil {
		return none[inputControlReply](), c.responseFailure(cafleetErrorOf(err), operation.input)
	}
	return c.answeredRefusal(operation.input), nil
}

func (c *inputCoordinator) permissionReceived(ctx context.Context, conn *dbConn, request inputControlRequest, observation *observationController, deadline time.Time) (opt[inputControlReply], *inputFailure) {
	operation, ok := request.Operation.(permissionRespondOperation)
	if !ok {
		panic("permission response route selects its semantic operation")
	}
	reply, err := c.respond(ctx, conn, request, operation.input, observation, deadline)
	if err != nil {
		return none[inputControlReply](), c.responseFailure(cafleetErrorOf(err), operation.input)
	}
	return reply, nil
}

func (c *inputCoordinator) respond(ctx context.Context, conn *dbConn, request inputControlRequest, input permissionRespondInput, observation *observationController, deadline time.Time) (opt[inputControlReply], error) {
	deadline = boundedByInherited(ctx, deadline)
	if err := runtimeCheckDeadline(c.now(), deadline); err != nil {
		return none[inputControlReply](), err
	}
	if err := c.validateRespondIdentity(request, input); err != nil {
		return none[inputControlReply](), err
	}
	if reply := c.answeredRefusal(input); reply.set {
		return reply, nil
	}
	if err := request.Identity.validate(c.run); err != nil {
		return none[inputControlReply](), err
	}
	if reason, blocked := c.permissionBlocker(observation).get(); blocked {
		return none[inputControlReply](), runtimeError("permission request is blocked: " + reason)
	}
	current := c.currentRequest
	if current == nil {
		return none[inputControlReply](), runtimeError("current permission token unavailable")
	}
	binding := current.request
	if err := ensure(current.token == input.RequestToken && binding.interactionID == input.InteractionID && binding.eventID == input.EventID, "permission response token identity changed"); err != nil {
		return none[inputControlReply](), err
	}
	if reason, mismatched := answerKindMismatch(binding.observed.classification, input.Key).get(); mismatched {
		return some[inputControlReply](c.rejectedResponse(input, reason)), nil
	}
	var matched *answerMatch
	for index := range binding.observed.answerMatches {
		if binding.observed.answerMatches[index].plan.capability.key == input.Key {
			matched = &binding.observed.answerMatches[index]
			break
		}
	}
	if matched == nil {
		switch key := input.Key.(type) {
		case answerKeyDecision:
			return none[inputControlReply](), runtimeError("selected permission decision is unsupported")
		case answerKeyChoice:
			return some[inputControlReply](c.rejectedResponse(input, fmt.Sprintf("choice %s has no tested answer for this question", key.choice))), nil
		default:
			panic("invalid answerKey")
		}
	}
	selection := &permissionSelection{request: input, binding: binding, gapCount: observation.gapCount, plan: matched.plan}
	if err := c.validatePermissionBinding(selection, observation); err != nil {
		return none[inputControlReply](), err
	}
	if _, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (struct{}, error) {
		return inTransaction(conn, transactionDeferred, func() (struct{}, error) {
			if err := c.validateLive(ctx, conn); err != nil {
				return struct{}{}, err
			}
			if err := c.validatePermissionAssociation(ctx, conn, selection); err != nil {
				return struct{}{}, err
			}
			return struct{}{}, runtimeCheckDeadline(c.now(), deadline)
		})
	}); err != nil {
		return none[inputControlReply](), err
	}
	if c.pending != nil {
		return some[inputControlReply](c.zeroByteResponse(input, submissionStatusHeld, "another programmatic input is pending")), nil
	}
	if err := ensure(len(c.operations) < receiptCacheCapacity, "input receipt cache is at capacity"); err != nil {
		return none[inputControlReply](), err
	}
	first, err := selectedAnswerStage(&selection.plan, 0)
	if err != nil {
		return none[inputControlReply](), err
	}
	answer, err := canonicalJSON(jsonValue(permissionRespondInputWireOf(input)))
	if err != nil {
		panic("a permission answer serializes: " + err.Error())
	}
	c.setPending(&pendingInput{
		reservation: none[attemptKey](),
		request: attemptRequest{
			operationKey:  fmt.Sprintf("answer:%d:%d", c.run.runID, input.InteractionID),
			kind:          attemptKindAnswer{},
			payloadDigest: sha256Hex([]byte(answer)),
		},
		bytes:    first.bytes,
		delivery: pendingDeliveryPermission{selection: selection},
		deadline: deadline,
		phase:    inputPhaseAwaitReservation,
	})
	return none[inputControlReply](), nil
}

// permissionRespondInputWireOf is the serde encoding of a permission answer.
func permissionRespondInputWireOf(input permissionRespondInput) permissionRespondInputWire {
	answer := answerWireOf(input.Key)
	return permissionRespondInputWire{
		InteractionID:      input.InteractionID,
		EventID:            input.EventID,
		DirectorMemberID:   input.DirectorMemberID,
		ExpectedRunID:      input.ExpectedRunID,
		ExpectedOwnerEpoch: input.ExpectedOwnerEpoch,
		RequestToken:       input.RequestToken,
		Decision:           answer.Decision,
		Choice:             answer.Choice,
	}
}

func (c *inputCoordinator) permissionReply(input permissionRespondInput, submission submissionResult) permissionResponseResult {
	return permissionResponseResult{
		Submission:    submission,
		MemberID:      c.run.memberID,
		RunID:         c.run.runID,
		OwnerEpoch:    input.ExpectedOwnerEpoch,
		InteractionID: input.InteractionID,
		EventID:       input.EventID,
		Key:           input.Key,
	}
}

func (c *inputCoordinator) rejectedResponse(input permissionRespondInput, reason string) permissionResponseResult {
	return c.zeroByteResponse(input, submissionStatusRejected, reason)
}

func (c *inputCoordinator) zeroByteResponse(input permissionRespondInput, status submissionStatus, reason string) permissionResponseResult {
	return c.permissionReply(input, submissionResult{
		Status:    status,
		InputID:   none[int64](),
		Stage:     submissionStageZeroBytes,
		OutputSeq: none[uint64](),
		Reason:    some(reason),
		Delivery:  none[deliveryProjection](),
	})
}

func (c *inputCoordinator) runFailure(source *cafleetError, interactionID, inputID opt[int64], operationKey opt[string]) *inputFailure {
	return inputContext{runID: some(c.run.runID), interactionID: interactionID, inputID: inputID, operationKey: operationKey}.failure(source)
}

func (c *inputCoordinator) responseFailure(source *cafleetError, input permissionRespondInput) *inputFailure {
	return c.runFailure(source, some(input.InteractionID), none[int64](), none[string]())
}

// inputContext is the pending attempt's failure context; a permission answer has no receipt, so
// it names only its interaction.
func (c *inputCoordinator) inputContext() inputContext {
	pending := c.pending
	if pending == nil {
		return inputContext{runID: some(c.run.runID)}
	}
	if selection := pending.permission(); selection != nil {
		return inputContext{runID: some(c.run.runID), interactionID: some(selection.request.InteractionID)}
	}
	return inputContext{runID: some(c.run.runID), inputID: pending.receiptID(), operationKey: some(pending.request.operationKey)}
}

func (c *inputCoordinator) failure(source *cafleetError) *inputFailure {
	return c.inputContext().failure(source)
}

func (c *inputCoordinator) effectiveDeadline(ctx context.Context, deadline time.Time) (time.Time, error) {
	pending := c.pending
	if pending == nil {
		return time.Time{}, runtimeError("input writer is absent")
	}
	if pending.deadline.Before(deadline) {
		deadline = pending.deadline
	}
	effective := boundedByInherited(ctx, deadline)
	if err := runtimeCheckDeadline(c.now(), effective); err != nil {
		return time.Time{}, err
	}
	return effective, nil
}

func (c *inputCoordinator) validateLive(ctx context.Context, conn *dbConn) error {
	if err := ensure(!c.manualAccountingFailed && !c.inputFenced, "manual accounting failure fenced input"); err != nil {
		return err
	}
	return c.validateLiveRun(ctx, conn)
}

func (c *inputCoordinator) validateLiveRun(ctx context.Context, conn *dbConn) error {
	current, err := revalidateRunSnapshot(ctx, conn, c.run, c.placement, snapshotPolicyLive, snapshotMessages{
		runChanged:       "input coordinator current run changed",
		placementChanged: "input member placement changed",
	})
	if err != nil {
		return err
	}
	return c.lock.validate(ctx, conn, current.owner())
}

func (c *inputCoordinator) validateReceiptOwner(ctx context.Context, conn *dbConn) error {
	current, err := validateOwner(ctx, conn, c.run.owner())
	if err != nil {
		return err
	}
	if err := ensure(jsonEqual(stableRunIdentity(current), stableRunIdentity(c.run)), "receipt current run identity changed"); err != nil {
		return err
	}
	return c.lock.validate(ctx, conn, current.owner())
}

func (c *inputCoordinator) validateReceiptCache(input inputOperation) error {
	if err := ensure(input.runID == c.run.runID, "receipt for this run required"); err != nil {
		return err
	}
	if retained, ok := c.operations[attemptKeyReceipt{id: input.inputID}]; ok {
		return ensure(attemptRecordReceiptOf(retained.operation).sameIdentity(input), "receipt cache identity changed")
	}
	return nil
}

func (c *inputCoordinator) receiptProjection(operation inputOperation) (submissionResult, error) {
	if err := c.validateReceiptCache(operation); err != nil {
		return submissionResult{}, err
	}
	result := c.result(operation)
	disposition, disposed := operation.disposition.get()
	if _, retained := c.operations[attemptKeyReceipt{id: operation.inputID}]; retained {
		point, err := c.retainedCheckpoint(attemptKeyReceipt{id: operation.inputID})
		if err != nil {
			return submissionResult{}, err
		}
		result.Stage = point.stage
	} else if operation.state == inputStateSettled && (!disposed || disposition != inputDispositionSubmitted) {
		result.Stage = submissionStageUnknown
	}
	return result, nil
}

func (c *inputCoordinator) receiptReceived(ctx context.Context, conn *dbConn, action receiptAction, deadline time.Time) (inputControlReply, error) {
	if retire, ok := action.(receiptRetire); ok {
		return c.retireReceived(ctx, conn, retire.InputID, deadline)
	}
	deadline = boundedByInherited(ctx, deadline)
	if err := runtimeCheckDeadline(c.now(), deadline); err != nil {
		return nil, err
	}
	inputID, operationKey := none[int64](), none[string]()
	reply, err := c.receipt(ctx, conn, action, deadline, &inputID, &operationKey)
	if err != nil {
		return nil, c.runFailure(cafleetErrorOf(err), none[int64](), inputID, operationKey).asError()
	}
	return reply, nil
}

func (c *inputCoordinator) receipt(ctx context.Context, conn *dbConn, action receiptAction, deadline time.Time, inputID *opt[int64], operationKey *opt[string]) (inputControlReply, error) {
	switch action := action.(type) {
	case receiptQuery:
		*operationKey = some(action.OperationKey)
		if err := validateOperationKey(action.OperationKey); err != nil {
			return nil, err
		}
		return withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (inputControlReply, error) {
			return inTransaction(conn, transactionDeferred, func() (inputControlReply, error) {
				if err := c.validateReceiptOwner(ctx, conn); err != nil {
					return nil, err
				}
				found, err := findInput(ctx, conn, c.run.runID, action.OperationKey)
				if err != nil {
					return nil, err
				}
				var reply inputControlReply
				if input, present := found.get(); present {
					*inputID = some(input.inputID)
					submission, err := c.receiptProjection(input)
					if err != nil {
						return nil, err
					}
					reply = receiptPresent{InputID: input.inputID, Submission: submission}
				} else {
					for key, retained := range c.operations {
						if _, receipt := key.(attemptKeyReceipt); receipt && attemptRecordReceiptOf(retained.operation).operationKey == action.OperationKey {
							return nil, runtimeError("retained input has no durable reservation")
						}
					}
					reply = receiptMissing{}
				}
				return reply, runtimeCheckDeadline(c.now(), deadline)
			})
		})
	case receiptReconcile:
		*inputID = some(action.InputID)
		if err := ensure(action.InputID > 0, "positive reconciliation input ID required"); err != nil {
			return nil, err
		}
		if err := ensure(strings.TrimSpace(action.EvidenceRef) != "", "nonempty reconciliation evidence required"); err != nil {
			return nil, err
		}
		settled, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (inputOperation, error) {
			return inTransaction(conn, transactionImmediate, func() (inputOperation, error) {
				return c.reconcileInTransaction(ctx, conn, action, deadline, operationKey)
			})
		})
		if err != nil {
			return nil, err
		}
		if retained, ok := c.operations[attemptKeyReceipt{id: action.InputID}]; ok {
			retained.operation = attemptRecordReceipt{operation: settled}
		}
		c.queue = slices.DeleteFunc(c.queue, func(queued queuedInput) bool { return queued.inputID == action.InputID })
		if err := runtimeCheckDeadline(c.now(), deadline); err != nil {
			return nil, err
		}
		return receiptSettled{InputID: action.InputID}, nil
	case receiptRetire:
		panic("retirement handled before receipt transaction")
	default:
		panic("invalid receiptAction")
	}
}

func (c *inputCoordinator) reconcileInTransaction(ctx context.Context, conn *dbConn, action receiptReconcile, deadline time.Time, operationKey *opt[string]) (inputOperation, error) {
	if err := c.validateReceiptOwner(ctx, conn); err != nil {
		return inputOperation{}, err
	}
	input, err := getInput(ctx, conn, action.InputID)
	if err != nil {
		return inputOperation{}, err
	}
	*operationKey = some(input.operationKey)
	if err := c.validateReceiptCache(input); err != nil {
		return inputOperation{}, err
	}
	if err := ensure(c.pending == nil, "reconciliation requires no active input writer"); err != nil {
		return inputOperation{}, err
	}
	if retained, ok := c.operations[attemptKeyReceipt{id: action.InputID}]; ok {
		if err := ensure(!retained.leased && !retained.attemptAvailable, "reconciliation requires an unleased consumed attempt"); err != nil {
			return inputOperation{}, err
		}
	}
	if err := ensure(input.state == action.Expected, "input state changed"); err != nil {
		return inputOperation{}, err
	}
	settled := input
	if input.state == inputStateSettled {
		if err := ensure(input.disposition == some(action.Disposition), "settled input disposition changed"); err != nil {
			return inputOperation{}, err
		}
	} else if settled, err = settleOwnedInputInTransaction(ctx, conn, c.run.owner(), input, action.Disposition, action.EvidenceRef); err != nil {
		return inputOperation{}, err
	}
	return settled, runtimeCheckDeadline(c.now(), deadline)
}

func (c *inputCoordinator) retireReceived(ctx context.Context, conn *dbConn, inputID int64, deadline time.Time) (inputControlReply, error) {
	deadline = boundedByInherited(ctx, deadline)
	if err := runtimeCheckDeadline(c.now(), deadline); err != nil {
		return nil, err
	}
	operationKey := none[string]()
	reply, err := c.retire(ctx, conn, inputID, deadline, &operationKey)
	if err != nil {
		return nil, c.runFailure(cafleetErrorOf(err), none[int64](), some(inputID), operationKey).asError()
	}
	return reply, nil
}

func (c *inputCoordinator) retire(ctx context.Context, conn *dbConn, inputID int64, deadline time.Time, operationKey *opt[string]) (inputControlReply, error) {
	if err := ensure(inputID > 0, "positive retirement input ID required"); err != nil {
		return nil, err
	}
	if _, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (struct{}, error) {
		return inTransaction(conn, transactionDeferred, func() (struct{}, error) {
			if err := c.validateReceiptOwner(ctx, conn); err != nil {
				return struct{}{}, err
			}
			input, err := getInput(ctx, conn, inputID)
			if err != nil {
				return struct{}{}, err
			}
			*operationKey = some(input.operationKey)
			if err := c.validateReceiptCache(input); err != nil {
				return struct{}{}, err
			}
			if err := ensure(input.state == inputStateSettled, "retirement requires this run's settled input"); err != nil {
				return struct{}{}, err
			}
			if err := ensure(c.pending == nil, "retirement requires no active input writer"); err != nil {
				return struct{}{}, err
			}
			if retained, ok := c.operations[attemptKeyReceipt{id: inputID}]; ok {
				if err := ensure(!retained.leased && !retained.attemptAvailable, "retirement requires an unleased consumed attempt"); err != nil {
					return struct{}{}, err
				}
			}
			return struct{}{}, runtimeCheckDeadline(c.now(), deadline)
		})
	}); err != nil {
		return nil, err
	}
	if err := runtimeCheckDeadline(c.now(), deadline); err != nil {
		return nil, err
	}
	delete(c.operations, attemptKeyReceipt{id: inputID})
	return receiptRetired{InputID: inputID}, nil
}

func composerEligible(observation *observationController) bool {
	status := observation.status()
	_, diagnosed := status.diagnostic.get()
	return status.eligibility == inputEligibility(inputEligibilityComposerEmpty{}) &&
		status.visibility == visibilityNotVisible && !status.retainedDraft && !diagnosed
}

func (c *inputCoordinator) verifiedComposer(operation composerOperation) bool {
	return c.services.catalog.verifiedComposer(c.backend, operation)
}

func (c *inputCoordinator) attemptRequirement(operation composerOperation, stages int) time.Duration {
	if stages < 1 {
		panic("a delivery has stages")
	}
	requirement := c.stageSettle()*time.Duration(stages-1) + stageWriteMargin
	if c.verifiedComposer(operation) {
		requirement += runtimeInputPostSubmitVerification
	}
	return requirement
}

// unresolvedOrdinary reports that the FIFO is stopped: a prompt or exec of the run is in progress
// or uncertain.
func (c *inputCoordinator) unresolvedOrdinary(ctx context.Context, conn *dbConn) (bool, error) {
	return hasUnresolvedOrdinaryAttempt(ctx, conn, c.run.runID)
}

func (c *inputCoordinator) result(operation inputOperation) submissionResult {
	outcome, durable := durableOutcome(operation).get()
	if !durable {
		outcome = submissionOutcome{status: submissionStatusHeld, stage: submissionStageZeroBytes, reason: some("input requires live proven-zero revalidation")}
	}
	stage := outcome.stage
	outputSeq := none[uint64]()
	if _, retained := c.operations[attemptKeyReceipt{id: operation.inputID}]; retained {
		point, err := c.retainedCheckpoint(attemptKeyReceipt{id: operation.inputID})
		if err != nil {
			panic("present input cache supplies its checkpoint")
		}
		outputSeq = some(point.outputSeq)
		if (operation.state == inputStateInProgress || operation.state == inputStateUncertain) && point.stage != submissionStageZeroBytes {
			stage = point.stage
		}
	}
	return submissionResult{
		Status:    outcome.status,
		InputID:   some(operation.inputID),
		Stage:     stage,
		OutputSeq: outputSeq,
		Reason:    outcome.reason,
		Delivery:  some(operation.delivery()),
	}
}

// attemptResult is the submission result of an attempt's record. A receipt-free sweep preview
// reports its state with no input identity or delivery projection.
func (c *inputCoordinator) attemptResult(key attemptKey, record attemptRecord) submissionResult {
	var free receiptFreeRecord
	switch record := record.(type) {
	case attemptRecordReceipt:
		return c.result(record.operation)
	case attemptRecordReceiptFree:
		free = record.record
	default:
		panic("invalid attemptRecord")
	}
	var status submissionStatus
	var stage submissionStage
	reason := none[string]()
	switch free.state {
	case inputStateAccepted:
		status, stage = submissionStatusAccepted, submissionStageSubmitted
	case inputStateInProgress, inputStateUncertain:
		status, stage = submissionStatusUncertain, submissionStageUnknown
		reason = optAsStr(free.stopCause)
	case inputStateReady:
		status, stage, reason = submissionStatusHeld, submissionStageZeroBytes, some("input requires live proven-zero revalidation")
	case inputStateSettled:
		panic("a receipt-free attempt is never settled")
	default:
		panic("invalid inputState")
	}
	outputSeq := none[uint64]()
	if _, retained := c.operations[key]; retained {
		point, err := c.retainedCheckpoint(key)
		if err != nil {
			panic("present attempt supplies its checkpoint")
		}
		outputSeq = some(point.outputSeq)
		if (free.state == inputStateInProgress || free.state == inputStateUncertain) && point.stage != submissionStageZeroBytes {
			stage = point.stage
		}
	}
	return submissionResult{Status: status, InputID: none[int64](), Stage: stage, OutputSeq: outputSeq, Reason: reason, Delivery: none[deliveryProjection]()}
}

func (c *inputCoordinator) handleReceived(ctx context.Context, conn *dbConn, request inputControlRequest, observation *observationController, deadline time.Time) (opt[inputControlReply], *inputFailure) {
	return c.receive(ctx, conn, request, observation, deadline)
}

func heldSubmission(reason string) (opt[inputControlReply], error) {
	return some[inputControlReply](submissionResultHeld(reason)), nil
}

// handlePreview starts the runner's own sweep preview: it holds no durable receipt, so every
// obstacle leaves the message pending for a later sweep instead of queueing it.
func (c *inputCoordinator) handlePreview(ctx context.Context, conn *dbConn, message messageRecord, observation *observationController, deadline time.Time) (opt[inputControlReply], *inputFailure) {
	reply, err := c.preview(ctx, conn, message, observation, deadline)
	if err != nil {
		return none[inputControlReply](), c.runFailure(cafleetErrorOf(err), none[int64](), none[int64](), none[string]())
	}
	return reply, nil
}

func (c *inputCoordinator) preview(ctx context.Context, conn *dbConn, message messageRecord, observation *observationController, deadline time.Time) (opt[inputControlReply], error) {
	if err := runtimeCheckDeadline(c.now(), deadline); err != nil {
		return none[inputControlReply](), err
	}
	if err := ensure(message.toMemberID == c.run.memberID, "a sweep preview addresses the run's member"); err != nil {
		return none[inputControlReply](), err
	}
	literal := canonicalPreviewLine(message)
	status := observation.status()
	if blocker, blocked := ordinaryBlockerOf(status).get(); blocked {
		switch blocker {
		case ordinaryBlockerObserverUnavailable:
			return heldSubmission("observer_unavailable")
		case ordinaryBlockerInteraction:
			c.previewWait = nil
			return heldSubmission(interactionCause(status).asStr())
		default:
			panic("invalid ordinaryBlocker")
		}
	}
	if c.pending != nil || len(c.queue) > 0 || c.finalization != nil {
		return heldSubmission("earlier_input")
	}
	unresolved, err := withDatabaseDeadline(ctx, conn, deadline, c.unresolvedOrdinary)
	if err != nil {
		return none[inputControlReply](), err
	}
	if unresolved {
		return heldSubmission("earlier_input")
	}
	probe, acquired, err := paneLockTryAcquire(c.databaseURL, c.placement)
	if err != nil {
		return none[inputControlReply](), err
	}
	if !acquired {
		return heldSubmission("pane_input_busy")
	}
	probe.close()
	kind := attemptKindPreview{messageID: message.messageID}
	choice, err := c.selectDelivery(ctx, conn, observation, composerInput{operation: composerOperationPrompt, kind: kind, literal: literal}, deadline)
	if err != nil {
		return none[inputControlReply](), err
	}
	var selection deliverySelection
	switch choice := choice.(type) {
	case deliveryChoiceWaiting:
		c.watchPreview(observation, message.messageID)
		return heldSubmission("composer_busy")
	case deliveryChoicePlainLimit:
		return none[inputControlReply](), runtimeError("a preview renders its pointer form within the plain limit: " + choice.cause)
	case deliveryChoiceReady:
		selection = choice.selection
	default:
		panic("invalid deliveryChoice")
	}
	if c.verifiedComposer(composerOperationPrompt) && max(deadline.Sub(c.now()), 0) < c.attemptRequirement(composerOperationPrompt, len(selection.stages)) {
		return heldSubmission("delivery_budget_exhausted")
	}
	if _, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (struct{}, error) {
		return struct{}{}, c.validateLive(ctx, conn)
	}); err != nil {
		return none[inputControlReply](), err
	}
	c.previewWait = nil
	c.setPending(&pendingInput{
		reservation: none[attemptKey](),
		request: attemptRequest{
			operationKey:  fmt.Sprintf("preview:%d:%d", c.run.runID, message.messageID),
			kind:          kind,
			payloadDigest: sha256Hex([]byte(literal)),
		},
		bytes:    selection.stages[0],
		delivery: pendingDeliveryComposer{composer: newComposerDelivery(selection, literal, composerOperationPrompt, nil, nil)},
		deadline: deadline,
		phase:    inputPhaseAwaitReservation,
	})
	return none[inputControlReply](), nil
}

func newComposerDelivery(selection deliverySelection, literal string, operation composerOperation, pane *paneLock, deferred *deferredAttempt) *composerDelivery {
	return &composerDelivery{
		method:      selection.method,
		reason:      selection.reason,
		stages:      selection.stages,
		settleUntil: none[time.Time](),
		stop:        none[composerStop](),
		pane:        pane,
		literal:     literal,
		operation:   operation,
		deferred:    deferred,
	}
}

func (c *inputCoordinator) receive(ctx context.Context, conn *dbConn, request inputControlRequest, observation *observationController, deadline time.Time) (opt[inputControlReply], *inputFailure) {
	if _, respond := request.Operation.(permissionRespondOperation); respond {
		return c.permissionReceived(ctx, conn, request, observation, deadline)
	}
	knownInputID, knownKey := none[int64](), none[string]()
	reply, err := c.submission(ctx, conn, request, observation, deadline, &knownInputID, &knownKey)
	if err != nil {
		return none[inputControlReply](), c.runFailure(cafleetErrorOf(err), none[int64](), knownInputID, knownKey)
	}
	return reply, nil
}

func (c *inputCoordinator) submission(ctx context.Context, conn *dbConn, request inputControlRequest, observation *observationController, deadline time.Time, knownInputID *opt[int64], knownKey *opt[string]) (opt[inputControlReply], error) {
	if err := runtimeCheckDeadline(c.now(), deadline); err != nil {
		return none[inputControlReply](), err
	}
	expected := controlIdentityFromRun(c.run)
	if err := ensure(request.Identity.OwnerEpoch > 0, "positive input owner epoch required"); err != nil {
		return none[inputControlReply](), err
	}
	expected.OwnerEpoch = request.Identity.OwnerEpoch
	if err := ensure(request.Identity.equal(expected), "input request fixed identity mismatch"); err != nil {
		return none[inputControlReply](), err
	}
	var input submitInput
	var operation composerOperation
	switch submitted := request.Operation.(type) {
	case receiptOperation:
		switch action := submitted.action.(type) {
		case receiptQuery:
			*knownKey = some(action.OperationKey)
		case receiptRetire:
			*knownInputID = some(action.InputID)
		case receiptReconcile:
			*knownInputID = some(action.InputID)
		default:
			panic("invalid receiptAction")
		}
		if err := request.Identity.validate(c.run); err != nil {
			return none[inputControlReply](), err
		}
		reply, err := c.receiptReceived(ctx, conn, submitted.action, deadline)
		if err != nil {
			return none[inputControlReply](), err
		}
		return some(reply), nil
	case submitPromptOperation:
		input, operation = submitted.input, composerOperationPrompt
	case submitExecOperation:
		input, operation = submitted.input, composerOperationExec
	case permissionInspectOperation, permissionRespondOperation:
		panic("serve_input routes only prompt and exec submissions here")
	default:
		panic("invalid inputControlOperation")
	}
	*knownKey = some(input.OperationKey)
	matching := (operation == composerOperationPrompt && input.InputKind == inputKindPrompt) ||
		(operation == composerOperationExec && input.InputKind == inputKindExec)
	if err := ensure(matching, "composer submission operation and kind mismatch"); err != nil {
		return none[inputControlReply](), err
	}
	if err := validateOperationKey(input.OperationKey); err != nil {
		return none[inputControlReply](), err
	}
	if err := ensure(validInput(input.Literal), "composer submission requires one nonempty line without control characters"); err != nil {
		return none[inputControlReply](), err
	}
	found, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (opt[inputOperation], error) {
		identity, err := databaseIdentity(ctx, conn)
		if err != nil {
			return none[inputOperation](), err
		}
		if err := ensure(jsonEqual(identity, c.run.databaseIdentityJSON), "input database identity changed"); err != nil {
			return none[inputOperation](), err
		}
		return findInput(ctx, conn, c.run.runID, input.OperationKey)
	})
	if err != nil {
		return none[inputControlReply](), err
	}
	digest := inputPayloadDigest(c.run.runID, input)
	if retained, present := found.get(); present {
		*knownInputID = some(retained.inputID)
		if err := ensure(retained.kind == input.InputKind && retained.payloadDigest == digest, "conflicting input operation key reuse"); err != nil {
			return none[inputControlReply](), err
		}
		return some[inputControlReply](c.result(retained)), nil
	}
	if err := request.Identity.validate(c.run); err != nil {
		return none[inputControlReply](), err
	}
	unresolvedDeferred, err := withDatabaseDeadline(ctx, conn, deadline, c.unresolvedOrdinary)
	if err != nil {
		return none[inputControlReply](), err
	}
	compose := composerInput{operation: operation, kind: attemptKindReceipt(input.InputKind), literal: input.Literal}
	deferReason, rejection, err := c.submissionDeferral(ctx, conn, observation, compose, unresolvedDeferred, deadline)
	if err != nil || rejection.set {
		return rejection, err
	}
	request2 := newInput{operationKey: input.OperationKey, kind: input.InputKind, payloadDigest: digest}
	if reason, deferred := deferReason.get(); deferred {
		reservation, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (inputReservation, error) {
			if err := c.validateLive(ctx, conn); err != nil {
				return inputReservation{}, err
			}
			return reserveInputWithProvenance(ctx, conn, c.run.owner(), request2, newReservation{selection: none[reservationSelection](), deferReason: some(reason)})
		})
		if err != nil {
			return none[inputControlReply](), err
		}
		if reservation.inserted {
			c.queue = append(c.queue, queuedInput{
				inputID:         reservation.operation.inputID,
				request:         request2,
				literal:         input.Literal,
				operation:       operation,
				attemptDeadline: none[time.Time](),
				obstacle:        "delivery_budget_exhausted",
				headSince:       none[time.Time](),
			})
		}
		*knownInputID = some(reservation.operation.inputID)
		return some[inputControlReply](c.result(reservation.operation)), nil
	}
	if err := ensure(len(c.operations) < receiptCacheCapacity, "input receipt cache is at capacity"); err != nil {
		return none[inputControlReply](), err
	}
	if _, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (struct{}, error) {
		return struct{}{}, c.validateLive(ctx, conn)
	}); err != nil {
		return none[inputControlReply](), err
	}
	choice, err := c.selectDelivery(ctx, conn, observation, compose, deadline)
	if err != nil {
		return none[inputControlReply](), err
	}
	var selection deliverySelection
	switch choice := choice.(type) {
	case deliveryChoiceReady:
		selection = choice.selection
	case deliveryChoicePlainLimit:
		return some[inputControlReply](submissionResultRejectedBeforeWrite(choice.cause)), nil
	case deliveryChoiceWaiting:
		return none[inputControlReply](), runtimeError("a busy composer queues the input before selection")
	default:
		panic("invalid deliveryChoice")
	}
	c.setPending(&pendingInput{
		reservation: none[attemptKey](),
		request:     attemptRequestReceipt(request2),
		bytes:       selection.stages[0],
		delivery:    pendingDeliveryComposer{composer: newComposerDelivery(selection, input.Literal, operation, nil, nil)},
		deadline:    deadline,
		phase:       inputPhaseAwaitReservation,
	})
	return none[inputControlReply](), nil
}

// submissionDeferral is why a new ordinary submission queues instead of starting, or a reply
// that ends it before any reservation.
func (c *inputCoordinator) submissionDeferral(ctx context.Context, conn *dbConn, observation *observationController, compose composerInput, unresolvedDeferred bool, deadline time.Time) (opt[string], opt[inputControlReply], error) {
	status := observation.status()
	if blocker, blocked := ordinaryBlockerOf(status).get(); blocked {
		switch blocker {
		case ordinaryBlockerObserverUnavailable:
			return none[string](), some[inputControlReply](submissionResultHeld("observer_unavailable")), nil
		case ordinaryBlockerInteraction:
			return some(interactionCause(status).asStr()), none[inputControlReply](), nil
		default:
			panic("invalid ordinaryBlocker")
		}
	}
	if c.pending != nil || len(c.queue) > 0 || c.finalization != nil || unresolvedDeferred {
		return some("earlier_input"), none[inputControlReply](), nil
	}
	choice, err := c.selectDelivery(ctx, conn, observation, compose, deadline)
	if err != nil {
		return none[string](), none[inputControlReply](), err
	}
	switch choice := choice.(type) {
	case deliveryChoiceWaiting:
		return some("composer_busy"), none[inputControlReply](), nil
	case deliveryChoicePlainLimit:
		return none[string](), some[inputControlReply](submissionResultRejectedBeforeWrite(choice.cause)), nil
	case deliveryChoiceReady:
		if c.verifiedComposer(compose.operation) && max(deadline.Sub(c.now()), 0) < c.attemptRequirement(compose.operation, len(choice.selection.stages)) {
			return some("delivery_budget_exhausted"), none[inputControlReply](), nil
		}
		return none[string](), none[inputControlReply](), nil
	default:
		panic("invalid deliveryChoice")
	}
}

// selectDelivery chooses profile delivery for a verified empty composer with an executable plan,
// and delivery fallback otherwise, rendering a fallback preview with its dismissal clause.
func (c *inputCoordinator) selectDelivery(ctx context.Context, conn *dbConn, observation *observationController, input composerInput, deadline time.Time) (deliveryChoice, error) {
	backend, known := codingAgentCodingAgent(c.run.backend)
	if !known {
		return nil, runtimeError("unknown input backend")
	}
	plan, planned := c.services.catalog.composerPlan(backend, input.operation)
	method, reason := deliveryMethodFallback, some("submission_plan_unavailable")
	if planned {
		if !composerEligible(observation) {
			return deliveryChoiceWaiting{}, nil
		}
		method, reason = deliveryMethodProfile, none[string]()
	}
	_, preview := input.kind.(attemptKindPreview)
	render := func(base string) string {
		if method == deliveryMethodFallback && preview {
			return fallbackPreviewLine(base)
		}
		return base
	}
	literal := render(input.literal)
	framing := literalFramingPlain
	if planned {
		if plan.literalFraming == literalFramingBracketedPaste && c.bracketedPaste {
			framing = literalFramingBracketedPaste
		} else if !plan.plainLiteralAllowed(literal) {
			previewKind, isPreview := input.kind.(attemptKindPreview)
			if !isPreview {
				return deliveryChoicePlainLimit{cause: plan.plainLimitRefusal(literal, c.run.backend)}, nil
			}
			message, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (messageRecord, error) {
				return getMessage(ctx, conn, previewKind.messageID)
			})
			if err != nil {
				return nil, err
			}
			literal = render(pointerPreviewLine(message))
		}
	}
	var stages [][]byte
	if planned {
		framed, err := plan.stages(literal, framing)
		if err != nil {
			return nil, err
		}
		if method == deliveryMethodFallback {
			stages = [][]byte{{fallbackEscape}, framed[0], {fallbackEnter}}
		} else {
			stages = [][]byte{framed[0], framed[1]}
		}
	} else {
		fallback, err := fallbackStages(input.operation.prefix(), literal)
		if err != nil {
			return nil, err
		}
		stages = fallback
	}
	return deliveryChoiceReady{selection: deliverySelection{method: method, reason: reason, stages: stages}}, nil
}

// deferredAttemptWriting reports that a started deferred attempt holds the pane and advances its
// stages on every tick.
func (c *inputCoordinator) deferredAttemptWriting() bool {
	if c.pending == nil {
		return false
	}
	composer := c.pending.composer()
	return composer != nil && composer.deferred != nil && composer.pane != nil
}

// sweepMayStart reports that the sweep may start a preview: no ordinary attempt is active and the
// FIFO is empty; a stopped FIFO is checked against storage when the preview is handled.
func (c *inputCoordinator) sweepMayStart() bool {
	return c.pending == nil && len(c.queue) == 0 && c.finalization == nil
}

func (c *inputCoordinator) hasDeferredWork() bool {
	return len(c.queue) > 0 || c.finalization != nil || (c.pending != nil && c.pending.deferred() != nil)
}

// currentInput is the attempt's current record: the stored row for prompt and exec, and the
// in-memory record of the sweep preview or the permission answer, which have no durable receipt.
func (c *inputCoordinator) currentInput(ctx context.Context, conn *dbConn, key attemptKey) (attemptRecord, error) {
	switch key := key.(type) {
	case attemptKeyReceipt:
		input, err := getInput(ctx, conn, key.id)
		if err != nil {
			return nil, err
		}
		return attemptRecordReceipt{operation: input}, nil
	case attemptKeySweepPreview, attemptKeyAnswer:
		return c.retained(key, "a receipt-free attempt retains its in-memory record").operation, nil
	default:
		panic("invalid attemptKey")
	}
}

func (c *inputCoordinator) sweepPreviewMessage() int64 {
	if c.pending != nil {
		if preview, ok := c.pending.request.kind.(attemptKindPreview); ok {
			return preview.messageID
		}
	}
	panic("the sweep preview attempt names its message")
}

// recordState moves an in-progress attempt to next; an accepted sweep preview stamps its
// message.
func (c *inputCoordinator) recordState(ctx context.Context, conn *dbConn, key attemptKey, expected, next inputState) (attemptRecord, error) {
	if id, receipt := attemptKeyReceiptOf(key); receipt {
		operation, err := recordInputState(ctx, conn, c.run.owner(), id, expected, next)
		if err != nil {
			return nil, err
		}
		return attemptRecordReceipt{operation: operation}, nil
	}
	if err := ensure(key == attemptKey(attemptKeySweepPreview{}), "only a sweep preview records an in-memory state"); err != nil {
		return nil, err
	}
	current, err := c.currentInput(ctx, conn, key)
	if err != nil {
		return nil, err
	}
	free, ok := current.(attemptRecordReceiptFree)
	if !ok {
		panic("a sweep preview keeps an in-memory record")
	}
	record := free.record
	if err := ensure(record.state == expected, "sweep preview state changed"); err != nil {
		return nil, err
	}
	switch next {
	case inputStateAccepted:
		if err := stampPreview(ctx, conn, c.run.owner(), c.sweepPreviewMessage()); err != nil {
			return nil, err
		}
	case inputStateReady:
		record.deliveryMethod = none[deliveryMethod]()
		record.deliveryReason = none[string]()
	case inputStateInProgress, inputStateUncertain, inputStateSettled:
		return nil, runtimeError("a sweep preview is accepted or returned to zero bytes")
	default:
		panic("invalid inputState")
	}
	record.state = next
	return attemptRecordReceiptFree{record: record}, nil
}

// recordUncertain records an in-progress attempt as uncertain. An uncertain sweep preview stays
// unnotified on a verified-composer run, so the sweep types it again; any other run stamps it
// first.
func (c *inputCoordinator) recordUncertain(ctx context.Context, conn *dbConn, key attemptKey, cause stopCause) (attemptRecord, error) {
	if id, receipt := attemptKeyReceiptOf(key); receipt {
		operation, err := recordInputUncertain(ctx, conn, c.run.owner(), id, cause)
		if err != nil {
			return nil, err
		}
		return attemptRecordReceipt{operation: operation}, nil
	}
	if err := ensure(key == attemptKey(attemptKeySweepPreview{}), "only a sweep preview records an in-memory uncertainty"); err != nil {
		return nil, err
	}
	current, err := c.currentInput(ctx, conn, key)
	if err != nil {
		return nil, err
	}
	free, ok := current.(attemptRecordReceiptFree)
	if !ok {
		panic("a sweep preview keeps an in-memory record")
	}
	record := free.record
	if err := ensure(record.state == inputStateInProgress, "sweep preview state changed"); err != nil {
		return nil, err
	}
	if !c.verifiedComposer(composerOperationPrompt) {
		if err := stampPreview(ctx, conn, c.run.owner(), c.sweepPreviewMessage()); err != nil {
			return nil, err
		}
	}
	record.state = inputStateUncertain
	record.stopCause = some(cause)
	return attemptRecordReceiptFree{record: record}, nil
}

// publishOutcome publishes a stopped attempt's result; a sweep preview publishes only an
// uncertain result, identified by its message.
func (c *inputCoordinator) publishOutcome(observation *observationController, key attemptKey, stage submissionStage, cause opt[stopCause]) {
	if id, receipt := attemptKeyReceiptOf(key); receipt {
		publishResult(observation, id, stage, cause)
		return
	}
	if key != attemptKey(attemptKeySweepPreview{}) {
		panic("a permission answer publishes no delivery result")
	}
	if stopped, set := cause.get(); set {
		retained, ok := c.operations[key]
		method, recorded := none[deliveryMethod]().get()
		if ok {
			method, recorded = attemptRecordDeliveryMethod(retained.operation).get()
		}
		if !recorded {
			panic("a stopped sweep preview recorded its method")
		}
		observation.publishDeliveryResult(previewDeliveryResultCause(c.sweepPreviewMessage(), method, stage.asStr(), stopped))
	}
}

func publishResult(observation *observationController, inputID int64, stage submissionStage, cause opt[stopCause]) {
	observation.publishDeliveryResult(inputDeliveryResultCause(inputID, stage.asStr(), cause))
}

// cancelDeferred settles queued or attempted zero-byte work as not submitted and publishes its
// result. When storage prevents the transaction, it stays a pending finalization that later
// ticks retry.
func (c *inputCoordinator) cancelDeferred(ctx context.Context, conn *dbConn, observation *observationController, inputID int64, expected inputState, reason string) error {
	operation, err := withDatabaseDeadline(ctx, conn, c.now().Add(deferredFinalization), func(ctx context.Context, conn *dbConn) (inputOperation, error) {
		return settleZeroByteDeferred(ctx, conn, c.run.owner(), inputID, expected, reason)
	})
	switch {
	case err != nil && storageFailure(err):
		c.recordDiagnostic(runnerDiagnosticSourceInputResult, fmt.Sprintf("input %d result is pending storage: %s", inputID, cafleetErrorOf(err).message()))
		c.finalization = &pendingFinalization{inputID: inputID, expected: expected, reason: reason}
		return nil
	case err != nil:
		c.inputFenced = true
		c.deferredFailed = true
		return err
	}
	if retained, ok := c.operations[attemptKeyReceipt{id: inputID}]; ok {
		retained.operation = attemptRecordReceipt{operation: operation}
		retained.attemptAvailable = false
	}
	publishResult(observation, inputID, submissionStageZeroBytes, none[stopCause]())
	return nil
}

// deferredTarget classifies the queued head's target before its first byte: none keeps it live,
// and a reason settles it. Ownership loss and storage failures return an error.
func (c *inputCoordinator) deferredTarget(ctx context.Context, conn *dbConn) (opt[string], error) {
	if err := ensure(!c.manualAccountingFailed && !c.inputFenced, "manual accounting failure fenced input"); err != nil {
		return none[string](), err
	}
	current, err := validateOwner(ctx, conn, c.run.owner())
	if err != nil {
		return none[string](), err
	}
	if err := ensure(current.status == runStatusRunning && current.ownerKind == ownerKindRunner, "input coordinator no longer owns a running run"); err != nil {
		return none[string](), err
	}
	if !jsonEqual(stableRunIdentity(current), stableRunIdentity(c.run)) || current.nativeVersion != c.run.nativeVersion {
		return some("target_changed"), nil
	}
	fleet, err := activeMemberFleet(ctx, conn, c.run.memberID)
	if err != nil {
		return none[string](), err
	}
	if fleet != some(c.run.fleetID) {
		return some("target_inactive"), nil
	}
	found, err := getMember(ctx, conn, c.run.memberID, c.run.fleetID)
	if err != nil {
		return none[string](), err
	}
	member, present := found.get()
	if !present {
		return some("target_inactive"), nil
	}
	if member.kind != memberKindMember || member.placement != c.placement {
		return some("target_changed"), nil
	}
	if err := c.lock.validate(ctx, conn, current.owner()); err != nil {
		return none[string](), err
	}
	return none[string](), nil
}

type queuedState struct {
	inputID int64
	state   inputState
}

// queueHousekeeping cancels a queued head whose target was removed, from one read transaction and
// independently of every queue-turn gate.
func (c *inputCoordinator) queueHousekeeping(ctx context.Context, conn *dbConn, observation *observationController) error {
	if c.inputFenced {
		return nil
	}
	if err := c.watchBlocked(ctx, conn, observation); err != nil {
		return err
	}
	if len(c.queue) == 0 {
		return nil
	}
	if pending := c.finalization; pending != nil {
		c.finalization = nil
		if err := c.cancelDeferred(ctx, conn, observation, pending.inputID, pending.expected, pending.reason); err != nil {
			return err
		}
		if c.finalization != nil {
			c.watchHead(observation)
			return nil
		}
	}
	entries := make([]int64, 0, len(c.queue))
	for _, queued := range c.queue {
		entries = append(entries, queued.inputID)
	}
	type housekeepingRead struct {
		sweep  []queuedState
		target opt[string]
	}
	read, err := withDatabaseDeadline(ctx, conn, c.now().Add(deferredFinalization), func(ctx context.Context, conn *dbConn) (housekeepingRead, error) {
		return inTransaction(conn, transactionDeferred, func() (housekeepingRead, error) {
			sweep := make([]queuedState, 0, len(entries))
			for _, inputID := range entries {
				input, err := getInput(ctx, conn, inputID)
				if err != nil {
					return housekeepingRead{}, err
				}
				sweep = append(sweep, queuedState{inputID: inputID, state: input.state})
			}
			target, err := c.deferredTarget(ctx, conn)
			if err != nil {
				return housekeepingRead{}, err
			}
			return housekeepingRead{sweep: sweep, target: target}, nil
		})
	})
	switch {
	case err != nil && storageFailure(err):
		c.watchHead(observation)
		return nil
	case err != nil:
		return err
	}
	for _, entry := range read.sweep {
		if entry.state != inputStateReady {
			c.inputFenced = true
			c.deferredFailed = true
			return runtimeError(fmt.Sprintf("queued input %d changed to %s outside the runner queue", entry.inputID, entry.state.asStr()))
		}
	}
	if reason, set := read.target.get(); set && len(c.queue) > 0 {
		head := c.popQueueHead()
		return c.cancelDeferred(ctx, conn, observation, head.inputID, inputStateReady, reason)
	}
	c.watchHead(observation)
	return nil
}

func (c *inputCoordinator) popQueueHead() queuedInput {
	if len(c.queue) == 0 {
		panic("queued head remains")
	}
	head := c.queue[0]
	c.queue = c.queue[1:]
	return head
}

func (c *inputCoordinator) queueHead() *queuedInput {
	if len(c.queue) == 0 {
		panic("queued head remains")
	}
	return &c.queue[0]
}

// watchBlocked tracks queued input or a pending preview waiting behind the run's lowest uncertain
// ordinary input, and publishes one blocked input_delivery_stalled once that wait lasts the stall
// bound. A transient storage failure skips the pass and keeps the wait.
func (c *inputCoordinator) watchBlocked(ctx context.Context, conn *dbConn, observation *observationController) error {
	runID, memberID := c.run.runID, c.run.memberID
	queued := len(c.queue) > 0
	type blockedRead struct {
		blocker opt[int64]
		waiter  bool
	}
	read, err := withDatabaseDeadline(ctx, conn, c.now().Add(deferredFinalization), func(ctx context.Context, conn *dbConn) (blockedRead, error) {
		return inTransaction(conn, transactionDeferred, func() (blockedRead, error) {
			blocker, err := firstUncertainOrdinaryInput(ctx, conn, runID)
			if err != nil {
				return blockedRead{}, err
			}
			_, blocked := blocker.get()
			waiter := false
			if blocked {
				waiter = queued
				if !waiter {
					preview, err := oldestPendingPreview(ctx, conn, memberID)
					if err != nil {
						return blockedRead{}, err
					}
					_, waiter = preview.get()
				}
			}
			return blockedRead{blocker: blocker, waiter: waiter}, nil
		})
	})
	switch {
	case err != nil && storageFailure(err):
		return nil
	case err != nil:
		return err
	}
	blocker, blocked := read.blocker.get()
	if !blocked || !read.waiter {
		c.blockedWait = nil
		return nil
	}
	now := c.now()
	if c.blockedWait == nil || c.blockedWait.blockerInputID != blocker {
		c.blockedWait = &blockedWait{blockerInputID: blocker, since: now}
	}
	wait := c.blockedWait
	if wait.stalled || now.Sub(wait.since) < c.stallBound {
		return nil
	}
	wait.stalled = true
	observation.publishDeliveryStalled(blockedRecord(blocker, wait.since, now, c.services.clock))
	return nil
}

// watchHead publishes one input_delivery_stalled for a head that has started no attempt within
// the stall bound of becoming the head; time an interaction or an uncertain blocker holds it does
// not count.
func (c *inputCoordinator) watchHead(observation *observationController) {
	now := c.now()
	interaction := ordinaryBlockerOf(observation.status()) == some(ordinaryBlockerInteraction)
	blocked := c.blockedWait != nil
	if len(c.queue) == 0 {
		return
	}
	head := &c.queue[0]
	if interaction || blocked {
		head.headSince = none[time.Time]()
		head.stalled = false
		return
	}
	since, set := head.headSince.get()
	if !set {
		since = now
		head.headSince = some(now)
	}
	if head.stalled || now.Sub(since) < c.stallBound {
		return
	}
	head.stalled = true
	observation.publishDeliveryStalled(stalledRecord(some(head.inputID), none[int64](), since, now, c.services.clock))
}

// watchPreview publishes one input_delivery_stalled for the sweep's selected preview once the
// composer has held it for the stall bound.
func (c *inputCoordinator) watchPreview(observation *observationController, messageID int64) {
	now := c.now()
	if c.previewWait == nil || c.previewWait.messageID != messageID {
		c.previewWait = &previewWait{messageID: messageID, since: now}
	}
	wait := c.previewWait
	if wait.stalled || now.Sub(wait.since) < c.stallBound {
		return
	}
	wait.stalled = true
	observation.publishDeliveryStalled(stalledRecord(none[int64](), some(messageID), wait.since, now, c.services.clock))
}

func (c *inputCoordinator) deferredTargetWithin(ctx context.Context, conn *dbConn, now time.Time) (opt[string], error) {
	return withDatabaseDeadline(ctx, conn, now.Add(deferredFinalization), c.deferredTarget)
}

// startDeferred starts the queued head once no interaction blocks it, or settles it when its
// attempt budget expires or its preview was consumed.
func (c *inputCoordinator) startDeferred(ctx context.Context, conn *dbConn, observation *observationController, deadline time.Time) error {
	if c.inputFenced {
		return nil
	}
	if pending := c.finalization; pending != nil {
		c.finalization = nil
		if err := c.cancelDeferred(ctx, conn, observation, pending.inputID, pending.expected, pending.reason); err != nil {
			return err
		}
		if c.finalization != nil {
			return nil
		}
	}
	if len(c.queue) == 0 {
		return nil
	}
	head := c.queue[0]
	inputID, kind, operation := head.inputID, attemptKindReceipt(head.request.kind), head.operation
	now := c.now()
	target, err := c.deferredTargetWithin(ctx, conn, now)
	switch {
	case err != nil && storageFailure(err):
		c.noteHeadObstacle("storage_unavailable")
		return nil
	case err != nil:
		return err
	}
	if reason, set := target.get(); set {
		c.popQueueHead()
		return c.cancelDeferred(ctx, conn, observation, inputID, inputStateReady, reason)
	}
	unresolved, err := withDatabaseDeadline(ctx, conn, now.Add(deferredFinalization), c.unresolvedOrdinary)
	switch {
	case err != nil && storageFailure(err):
		c.noteHeadObstacle("storage_unavailable")
		return nil
	case err != nil:
		return err
	case unresolved:
		return nil
	}
	blocker, blocked := ordinaryBlockerOf(observation.status()).get()
	if blocked && blocker == ordinaryBlockerInteraction {
		c.queueHead().attemptDeadline = none[time.Time]()
		return nil
	}
	if !blocked {
		choice, err := c.selectDelivery(ctx, conn, observation, composerInput{operation: operation, kind: kind, literal: head.literal}, deadline)
		if err != nil {
			return err
		}
		switch choice := choice.(type) {
		case deliveryChoiceReady:
			return c.attemptDeferred(ctx, conn, observation, deadline, now, choice.selection)
		case deliveryChoicePlainLimit:
			c.popQueueHead()
			return c.cancelDeferred(ctx, conn, observation, inputID, inputStateReady, "plain_limit_exceeded")
		case deliveryChoiceWaiting:
			c.queueHead().attemptDeadline = none[time.Time]()
			return nil
		default:
			panic("invalid deliveryChoice")
		}
	}
	queued := c.queueHead()
	queued.obstacle = "observer_unavailable"
	attemptDeadline := c.headAttemptDeadline(queued, now, deadline)
	if !now.Before(attemptDeadline) {
		c.popQueueHead()
		return c.cancelDeferred(ctx, conn, observation, inputID, inputStateReady, "observer_unavailable")
	}
	return nil
}

// headAttemptDeadline is the head's attempt deadline, starting its budget now when it has none.
func (c *inputCoordinator) headAttemptDeadline(head *queuedInput, now, deadline time.Time) time.Time {
	attemptDeadline, set := head.attemptDeadline.get()
	if !set {
		attemptDeadline = now.Add(deferredAttemptBudget)
		if deadline.Before(attemptDeadline) {
			attemptDeadline = deadline
		}
		head.attemptDeadline = some(attemptDeadline)
	}
	return attemptDeadline
}

// attemptDeferred starts the selected attempt for the queued head within its five-second budget.
func (c *inputCoordinator) attemptDeferred(ctx context.Context, conn *dbConn, observation *observationController, deadline, now time.Time, selection deliverySelection) error {
	head := c.queueHead()
	attemptDeadline := c.headAttemptDeadline(head, now, deadline)
	inputID, operation := head.inputID, head.operation
	if !now.Before(attemptDeadline) {
		obstacle := head.obstacle
		c.popQueueHead()
		return c.cancelDeferred(ctx, conn, observation, inputID, inputStateReady, obstacle)
	}
	target, err := c.deferredTargetWithin(ctx, conn, now)
	switch {
	case err != nil && storageFailure(err):
		c.noteHeadObstacle("storage_unavailable")
		return nil
	case err != nil:
		return err
	}
	if reason, set := target.get(); set {
		c.popQueueHead()
		return c.cancelDeferred(ctx, conn, observation, inputID, inputStateReady, reason)
	}
	pane, acquired, err := paneLockTryAcquire(c.databaseURL, c.placement)
	if err != nil {
		return err
	}
	if !acquired {
		c.noteHeadObstacle("pane_input_busy")
		return nil
	}
	if max(attemptDeadline.Sub(now), 0) < c.attemptRequirement(operation, len(selection.stages)) {
		pane.close()
		c.noteHeadObstacle("delivery_budget_exhausted")
		return nil
	}
	begun, err := withDatabaseDeadline(ctx, conn, attemptDeadline, func(ctx context.Context, conn *dbConn) (inputOperation, error) {
		return beginDeferredAttempt(ctx, conn, c.run.owner(), inputID, selection.method, selection.reason)
	})
	switch {
	case err != nil && storageFailure(err):
		pane.close()
		c.noteHeadObstacle("storage_unavailable")
		return nil
	case err != nil:
		pane.close()
		return err
	}
	queued := c.popQueueHead()
	key := attemptKeyReceipt{id: inputID}
	c.operations[key] = &retainedInput{
		operation: attemptRecordReceipt{operation: begun},
		bytes:     selection.stages[0],
		selection: &retainedSelectionComposer{method: selection.method, stages: selection.stages},
		stage:     submissionStageZeroBytes,
	}
	c.setPending(&pendingInput{
		reservation: some(attemptKey(key)),
		request:     attemptRequestReceipt(queued.request),
		bytes:       selection.stages[0],
		delivery: pendingDeliveryComposer{composer: newComposerDelivery(selection, queued.literal, queued.operation, pane,
			&deferredAttempt{queued: queued, obstacle: "delivery_budget_exhausted"})},
		deadline: attemptDeadline,
		phase:    inputPhaseAwaitBeforeWrite,
	})
	return c.checkpoint(key, inputBoundaryReserved, attemptDeadline)
}

// deferredPreflight handles a deferred attempt's interaction, expiry and obstacles before any
// write; true means the attempt ended or returned to the queue.
func (c *inputCoordinator) deferredPreflight(ctx context.Context, conn *dbConn, observation *observationController) (bool, error) {
	pending := c.pending
	if pending == nil {
		return false, nil
	}
	composer := pending.composer()
	if composer == nil || composer.deferred == nil {
		return false, nil
	}
	id, set := pending.receiptID().get()
	if !set {
		panic("a deferred attempt has its receipt")
	}
	retained, ok := c.operations[attemptKeyReceipt{id: id}]
	zeroBytes := ok && retained.stage == submissionStageZeroBytes && composer.writtenStages == 0
	if !zeroBytes {
		return false, nil
	}
	interaction := ordinaryBlockerOf(observation.status()) == some(ordinaryBlockerInteraction)
	composerBusy := !interaction && composer.requiresEmptyComposer() && !composerEligible(observation)
	now := c.now()
	expired := !now.Before(pending.deadline)
	requeue := interaction || composerBusy
	if !requeue && !expired {
		return false, nil
	}
	if requeue {
		_, err := withDatabaseDeadline(ctx, conn, now.Add(deferredFinalization), func(ctx context.Context, conn *dbConn) (inputOperation, error) {
			return recordInputState(ctx, conn, c.run.owner(), id, inputStateInProgress, inputStateReady)
		})
		switch {
		case err != nil && storageFailure(err):
			composer.deferred.obstacle = "storage_unavailable"
			return true, nil
		case err != nil:
			return false, err
		}
	}
	taken := c.takePending()
	defer taken.releasePane()
	deferred := taken.composer().deferred
	if requeue {
		delete(c.operations, attemptKeyReceipt{id: id})
		queued := deferred.queued
		queued.attemptDeadline = none[time.Time]()
		queued.headSince = none[time.Time]()
		c.queue = append([]queuedInput{queued}, c.queue...)
		return true, nil
	}
	if err := c.cancelDeferred(ctx, conn, observation, id, inputStateInProgress, deferred.obstacle); err != nil {
		return false, err
	}
	delete(c.operations, attemptKeyReceipt{id: id})
	return true, nil
}

// immediatePreflight queues an immediate ordinary request that has written zero bytes once an
// interaction or a composer that is not verified empty supersedes its selected method, replying
// queued.
func (c *inputCoordinator) immediatePreflight(ctx context.Context, conn *dbConn, observation *observationController, deadline time.Time) (opt[inputControlReply], error) {
	pending := c.pending
	if pending == nil {
		return none[inputControlReply](), nil
	}
	composer := pending.composer()
	if composer == nil {
		return none[inputControlReply](), nil
	}
	zeroBytes := composer.writtenStages == 0
	if key, set := pending.attempt().get(); set {
		retained, ok := c.operations[key]
		zeroBytes = zeroBytes && ok && retained.stage == submissionStageZeroBytes
	}
	if composer.deferred != nil || !zeroBytes {
		return none[inputControlReply](), nil
	}
	status := observation.status()
	insufficientBudget := c.verifiedComposer(composer.operation) &&
		max(pending.deadline.Sub(c.now()), 0) < c.attemptRequirement(composer.operation, len(composer.stages))
	var deferReason string
	blocker, blocked := ordinaryBlockerOf(status).get()
	switch {
	case blocked && blocker == ordinaryBlockerInteraction:
		deferReason = interactionCause(status).asStr()
	case !blocked && composer.requiresEmptyComposer() && !composerEligible(observation):
		deferReason = "composer_busy"
	case !blocked && insufficientBudget:
		deferReason = "delivery_budget_exhausted"
	default:
		return none[inputControlReply](), nil
	}
	taken := c.takePending()
	defer taken.releasePane()
	if _, preview := taken.request.kind.(attemptKindPreview); preview {
		delete(c.operations, attemptKeySweepPreview{})
		return some[inputControlReply](submissionResultHeld(deferReason)), nil
	}
	var operation inputOperation
	if id, receipt := taken.receiptID().get(); receipt {
		delete(c.operations, attemptKeyReceipt{id: id})
		requeued, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (inputOperation, error) {
			return requeueZeroByteImmediate(ctx, conn, c.run.owner(), id, deferReason)
		})
		if err != nil {
			return none[inputControlReply](), err
		}
		operation = requeued
	} else {
		reservation, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (inputReservation, error) {
			if err := c.validateLive(ctx, conn); err != nil {
				return inputReservation{}, err
			}
			return reserveInputWithProvenance(ctx, conn, c.run.owner(), taken.request.newInput(), newReservation{selection: none[reservationSelection](), deferReason: some(deferReason)})
		})
		if err != nil {
			return none[inputControlReply](), err
		}
		if !reservation.inserted {
			return some[inputControlReply](c.result(reservation.operation)), nil
		}
		operation = reservation.operation
	}
	takenComposer := taken.composer()
	c.queue = append([]queuedInput{{
		inputID:         operation.inputID,
		request:         taken.request.newInput(),
		literal:         takenComposer.literal,
		operation:       takenComposer.operation,
		attemptDeadline: none[time.Time](),
		obstacle:        "delivery_budget_exhausted",
		headSince:       none[time.Time](),
	}}, c.queue...)
	return some[inputControlReply](c.result(operation)), nil
}

func (c *inputCoordinator) finishDeferred(observation *observationController, operation inputOperation) {
	stage := submissionStageUnknown
	if retained, ok := c.operations[attemptKeyReceipt{id: operation.inputID}]; ok {
		stage = retained.stage
	}
	publishResult(observation, operation.inputID, stage, operation.stopCause)
}

func (c *inputCoordinator) retainedCheckpoint(attempt attemptKey) (inputCheckpoint, error) {
	retained, ok := c.operations[attempt]
	if !ok {
		return inputCheckpoint{}, runtimeError("retained input operation is absent")
	}
	return inputCheckpoint{attempt: attempt, stage: retained.stage, writtenBytes: retained.writtenBytes, outputSeq: retained.outputSeq}, nil
}

func (c *inputCoordinator) checkpoint(attempt attemptKey, boundary inputBoundary, deadline time.Time) error {
	point, err := c.retainedCheckpoint(attempt)
	if err != nil {
		return err
	}
	var result error
	if c.services.hooks != nil {
		result = c.services.hooks.checkpoint(boundary, point, deadline)
	}
	if result == nil {
		result = runtimeCheckDeadline(c.now(), deadline)
	}
	if result != nil && c.pending != nil {
		c.pending.phase = inputPhaseBlocked
	}
	return result
}

func (c *inputCoordinator) drainRequirement() drainRequirement {
	pending := c.pending
	switch {
	case pending != nil && pending.permission() != nil:
		return drainRequirementObservation
	case pending != nil && pending.composer() != nil && !pending.composer().requiresEmptyComposer():
		return drainRequirementNoBlockingInteraction
	default:
		return drainRequirementEmptyComposer
	}
}

func (c *inputCoordinator) freshDrain(t *transport, observation *observationController, deadline time.Time) (opt[uint64], error) {
	if c.pending != nil {
		if permission := c.pending.permission(); permission != nil {
			if err := c.validatePermissionBinding(permission, observation); err != nil {
				return none[uint64](), err
			}
		}
	}
	requirement := c.drainRequirement()
	proof := t.takeInputDrain()
	if proof == nil {
		return none[uint64](), nil
	}
	valid, err := validateTakenDrain(t, proof, observation, requirement, deadline)
	if err != nil {
		return none[uint64](), err
	}
	if !valid {
		c.noteObserverObstacle(observation)
		return none[uint64](), nil
	}
	seq, err := c.currentDrainSeq(proof, "input manual generation changed")
	if err != nil {
		return none[uint64](), err
	}
	return some(seq), nil
}

func (c *inputCoordinator) noteHeadObstacle(obstacle string) {
	c.queueHead().obstacle = obstacle
}

// noteDeferredObstacle records the zero-byte obstacle a started deferred attempt last met, which
// its expiry reports.
func (c *inputCoordinator) noteDeferredObstacle(obstacle string) {
	if c.pending == nil {
		return
	}
	if deferred := c.pending.deferred(); deferred != nil {
		deferred.obstacle = obstacle
	}
}

func (c *inputCoordinator) noteObserverObstacle(observation *observationController) {
	if ordinaryBlockerOf(observation.status()) == some(ordinaryBlockerObserverUnavailable) {
		c.noteDeferredObstacle("observer_unavailable")
	}
}

// validateTakenDrain reports false when the observed state does not yet permit the drain's
// requirement.
func validateTakenDrain(t *transport, proof *drainedInput, observation *observationController, requirement drainRequirement, deadline time.Time) (bool, error) {
	switch requirement {
	case drainRequirementObservation:
		return true, t.validatePermission(proof, deadline)
	case drainRequirementEmptyComposer:
		if err := t.validateInputDrain(proof, deadline); err != nil {
			return false, err
		}
		return composerEligible(observation), nil
	case drainRequirementNoBlockingInteraction:
		if err := t.validateObservationDrain(proof, deadline); err != nil {
			return false, err
		}
		_, blocked := ordinaryBlockerOf(observation.status()).get()
		return !blocked, nil
	default:
		panic("invalid drainRequirement")
	}
}

func (c *inputCoordinator) currentDrainSeq(proof *drainedInput, message string) (uint64, error) {
	if err := ensure(proof.humanGeneration == c.manualGeneration && !c.manualAccountingFailed, message); err != nil {
		return 0, err
	}
	return proof.outputSeq, nil
}

func (c *inputCoordinator) validateWriteNow(ctx context.Context, conn *dbConn, operation *pendingInput, deadline time.Time) (writeValidation, error) {
	effective, err := c.effectiveDeadline(ctx, deadline)
	if err != nil {
		return 0, err
	}
	if err := ensure(conn.isAutocommit(), "final input validation requires autocommit"); err != nil {
		return 0, err
	}
	_, err = withDatabaseBudget(ctx, conn, deadlineUntil(effective), func(ctx context.Context, conn *dbConn) (struct{}, error) {
		if err := conn.refuseBusyWait(); err != nil {
			return struct{}{}, err
		}
		if err := conn.begin(transactionImmediate); err != nil {
			return struct{}{}, err
		}
		validation := c.validateReservation(ctx, conn, operation, effective)
		return struct{}{}, mergeCleanup(validation, conn.rollbackIfOpen())
	})
	switch {
	case err == nil:
		if err := runtimeCheckDeadline(c.now(), effective); err != nil {
			return 0, err
		}
		return writeValidationReady, nil
	case cafleetErrorOf(err).kind == errorDatabaseBusy:
		if err := runtimeCheckDeadline(c.now(), effective); err != nil {
			return 0, err
		}
		return writeValidationContended, nil
	default:
		return 0, err
	}
}

func (c *inputCoordinator) validateReservation(ctx context.Context, conn *dbConn, operation *pendingInput, effective time.Time) error {
	if err := c.validateLive(ctx, conn); err != nil {
		return err
	}
	key, reserved := operation.attempt().get()
	if !reserved {
		return runtimeError("input reservation is absent")
	}
	retained, ok := c.operations[key]
	if !ok {
		return runtimeError("retained input reservation is absent")
	}
	current, err := c.currentInput(ctx, conn, key)
	if err != nil {
		return err
	}
	if err := ensure(current == retained.operation && attemptRecordState(current) == inputStateInProgress, "input reservation changed before write"); err != nil {
		return err
	}
	if stored, receipt := current.(attemptRecordReceipt); receipt {
		if err := ensure(stored.operation.runID == c.run.runID && stored.operation.operationKey == operation.request.operationKey &&
			stored.operation.payloadDigest == operation.request.payloadDigest &&
			attemptKindReceipt(stored.operation.kind) == operation.request.kind, "input reservation changed before write"); err != nil {
			return err
		}
	}
	if selection := operation.permission(); selection != nil {
		if err := c.validatePermissionAssociation(ctx, conn, selection); err != nil {
			return err
		}
	}
	return runtimeCheckDeadline(c.now(), effective)
}

// holdComposerPane holds the concrete pane-input lock for a composer write; false while another
// writer holds it, so the attempt writes nothing and retries.
func (c *inputCoordinator) holdComposerPane() (bool, error) {
	if c.pending == nil {
		return true, nil
	}
	composer := c.pending.composer()
	if composer == nil {
		return true, nil
	}
	if composer.pane != nil {
		return true, composer.pane.validateBinding(c.databaseURL, c.placement)
	}
	pane, acquired, err := paneLockTryAcquire(c.databaseURL, c.placement)
	if err != nil {
		return false, err
	}
	if acquired {
		composer.pane = pane
	} else if composer.deferred != nil {
		composer.deferred.obstacle = "pane_input_busy"
	}
	return acquired, nil
}

type writeLease struct {
	key       attemptKey
	proof     *drainedInput
	deadline  time.Time
	outputSeq uint64
}

func (c *inputCoordinator) acquireWriteGuard(ctx context.Context, conn *dbConn, t *transport, observation *observationController, deadline time.Time) (*inputGuard, bool, error) {
	lease, leased, err := c.writeLease(ctx, conn, t, observation, deadline)
	if err != nil || !leased {
		return nil, false, err
	}
	retained := c.retained(lease.key, "validated input remains retained")
	retained.outputSeq = lease.outputSeq
	retained.attemptAvailable = false
	retained.leased = true
	return &inputGuard{retained: retained, proof: lease.proof, deadline: lease.deadline, permit: guardPermitFresh, clock: c.services.clock}, true, nil
}

func (c *inputCoordinator) writeLease(ctx context.Context, conn *dbConn, t *transport, observation *observationController, deadline time.Time) (writeLease, bool, error) {
	if c.pending == nil || c.pending.phase != inputPhaseArmed {
		return writeLease{}, false, nil
	}
	effective, err := c.effectiveDeadline(ctx, deadline)
	if err != nil {
		return writeLease{}, false, err
	}
	held, err := c.holdComposerPane()
	if err != nil || !held {
		return writeLease{}, false, err
	}
	requirement := c.drainRequirement()
	pending := c.pending
	proof := t.takeInputDrain()
	if proof == nil {
		return writeLease{}, false, nil
	}
	permission := pending.permission()
	if permission != nil {
		if err := c.validatePermissionBinding(permission, observation); err != nil {
			return writeLease{}, false, err
		}
	}
	valid, err := validateTakenDrain(t, proof, observation, requirement, effective)
	if err != nil {
		return writeLease{}, false, err
	}
	if !valid {
		c.noteObserverObstacle(observation)
		return writeLease{}, false, nil
	}
	if permission != nil {
		if err := c.validateRetainedPermission(pending, permission); err != nil {
			return writeLease{}, false, err
		}
	}
	outputSeq, err := c.currentDrainSeq(proof, "input manual generation changed")
	if err != nil {
		return writeLease{}, false, err
	}
	key, reserved := pending.attempt().get()
	if !reserved {
		return writeLease{}, false, runtimeError("armed input has no reservation")
	}
	retained, ok := c.operations[key]
	if !ok {
		return writeLease{}, false, runtimeError("armed input reservation is absent")
	}
	if err := ensure(retained.attemptAvailable && !retained.leased, "input attempt permit is consumed"); err != nil {
		return writeLease{}, false, err
	}
	validation, err := c.validateWriteNow(ctx, conn, pending, effective)
	if err != nil {
		return writeLease{}, false, err
	}
	if validation == writeValidationContended {
		c.noteDeferredObstacle("storage_unavailable")
		return writeLease{}, false, nil
	}
	if err := runtimeCheckDeadline(c.now(), effective); err != nil {
		return writeLease{}, false, err
	}
	return writeLease{key: key, proof: proof, deadline: effective, outputSeq: outputSeq}, true, nil
}

func (c *inputCoordinator) validateRetainedPermission(pending *pendingInput, permission *permissionSelection) error {
	key, reserved := pending.attempt().get()
	if !reserved {
		return runtimeError("permission reservation absent")
	}
	retained, ok := c.operations[key]
	if !ok {
		return runtimeError("retained permission selection absent")
	}
	selection, isPermission := retained.selection.(*retainedSelectionPermission)
	if !isPermission {
		return runtimeError("permission stage target changed")
	}
	if err := ensure(selection.selection == permission, "permission retained selection changed"); err != nil {
		return err
	}
	if _, err := selectedAnswerStage(&selection.selection.plan, selection.nextStage); err != nil {
		return err
	}
	return ensure(selection.selection.plan.capability.requestRuleID == selection.selection.binding.observed.requestRuleID, "permission stage rule or choice changed")
}

func (c *inputCoordinator) advance(ctx context.Context, conn *dbConn, t *transport, pty ptyIo, observation *observationController, deadline time.Time) (opt[inputControlReply], *inputFailure) {
	c.outputSeq = t.snapshot().screenSeq
	context := c.inputContext()
	terminalBookkeeping := false
	reply, err := c.advancePhase(ctx, conn, t, pty, observation, deadline, &terminalBookkeeping)
	if err == nil {
		return reply, nil
	}
	source := cafleetErrorOf(err)
	if c.pending != nil {
		if composer := c.pending.composer(); composer != nil && composer.stopFailure != nil {
			primary := composer.stopFailure
			composer.stopFailure = nil
			source = primary.withDiagnostic(source.message())
		}
		if composer := c.pending.composer(); composer != nil {
			composer.releasePane()
		}
	}
	c.retireFailedDeferred(ctx, conn, observation)
	if !terminalBookkeeping {
		answered, failed := c.answerFailure(source)
		if failed == nil {
			return some(answered), nil
		}
		source = c.recordImmediateWriteFailure(ctx, conn, observation, deadline, failed)
	}
	if c.pending != nil {
		return none[inputControlReply](), c.failure(source)
	}
	return none[inputControlReply](), context.failure(source)
}

func (c *inputCoordinator) advancePhase(ctx context.Context, conn *dbConn, t *transport, pty ptyIo, observation *observationController, deadline time.Time, terminalBookkeeping *bool) (opt[inputControlReply], error) {
	if c.pending != nil && c.finalization == nil && len(c.queue) > 0 {
		inputID := c.queue[0].inputID
		reason, err := c.deferredTargetWithin(ctx, conn, c.now())
		switch {
		case err != nil && storageFailure(err):
			c.noteHeadObstacle("storage_unavailable")
		case err != nil:
			return none[inputControlReply](), err
		default:
			if cancelled, set := reason.get(); set {
				c.popQueueHead()
				if err := c.cancelDeferred(ctx, conn, observation, inputID, inputStateReady, cancelled); err != nil {
					return none[inputControlReply](), err
				}
			}
		}
	}
	if c.pending == nil {
		return none[inputControlReply](), c.startDeferred(ctx, conn, observation, deadline)
	}
	ended, err := c.deferredPreflight(ctx, conn, observation)
	if err != nil || ended {
		return none[inputControlReply](), err
	}
	if reply, err := c.immediatePreflight(ctx, conn, observation, deadline); err != nil || reply.set {
		return reply, err
	}
	rejection, err := c.refreshZeroDelivery(ctx, conn, observation)
	if err != nil {
		return none[inputControlReply](), err
	}
	if result, rejected := rejection.get(); rejected {
		return some[inputControlReply](result), nil
	}
	if c.pending == nil {
		return none[inputControlReply](), nil
	}
	if c.pending.deferred() != nil && c.pending.deadline.Before(deadline) {
		deadline = c.pending.deadline
	}
	effective, err := c.effectiveDeadline(ctx, deadline)
	if err != nil {
		return none[inputControlReply](), err
	}
	switch c.pending.phase {
	case inputPhaseBlocked:
		return none[inputControlReply](), nil
	case inputPhaseClosing:
		return c.writeCloseStage(ctx, conn, t, pty, observation, effective)
	case inputPhaseVerifying:
		return c.verifySubmit(ctx, conn, t, observation, effective)
	case inputPhaseAwaitReservation:
		return c.reservePending(ctx, conn, t, observation, effective)
	case inputPhaseAwaitBeforeWrite:
		return c.armPending(ctx, conn, t, observation, effective, terminalBookkeeping)
	case inputPhaseArmed:
		return c.writePendingStage(ctx, conn, t, observation, pty, effective, terminalBookkeeping)
	default:
		panic("invalid inputPhase")
	}
}

func (c *inputCoordinator) reservedAttempt() (attemptKey, error) {
	if c.pending != nil {
		if key, set := c.pending.attempt().get(); set {
			return key, nil
		}
	}
	return nil, runtimeError("input writer has no reservation")
}

func (c *inputCoordinator) refreshZeroDelivery(ctx context.Context, conn *dbConn, observation *observationController) (opt[submissionResult], error) {
	pending := c.pending
	if pending == nil {
		panic("a pending input remains")
	}
	composer := pending.composer()
	if composer == nil || composer.started || pending.phase == inputPhaseBlocked {
		return none[submissionResult](), nil
	}
	id, reserved := pending.attempt().get()
	deferred := composer.deferred != nil
	choice, err := c.selectDelivery(ctx, conn, observation, composerInput{operation: composer.operation, kind: pending.request.kind, literal: composer.literal}, pending.deadline)
	if err != nil {
		return none[submissionResult](), err
	}
	var selection deliverySelection
	switch choice := choice.(type) {
	case deliveryChoiceWaiting:
		return none[submissionResult](), nil
	case deliveryChoicePlainLimit:
		return c.rejectZeroByte(ctx, conn, observation, id, reserved, deferred, pending.deadline, choice.cause)
	case deliveryChoiceReady:
		selection = choice.selection
	default:
		panic("invalid deliveryChoice")
	}
	if reserved {
		if err := c.reselectZeroByte(ctx, conn, id, selection, pending.deadline); err != nil {
			return none[submissionResult](), err
		}
	}
	pending.bytes = selection.stages[0]
	composer.method = selection.method
	composer.reason = selection.reason
	composer.stages = selection.stages
	return none[submissionResult](), nil
}

func (c *inputCoordinator) rejectZeroByte(ctx context.Context, conn *dbConn, observation *observationController, id attemptKey, reserved, deferred bool, deadline time.Time, cause string) (opt[submissionResult], error) {
	rejection := submissionResultRejectedBeforeWrite(cause)
	if reserved {
		retained := c.retained(id, "a zero-byte input retains its receipt")
		if err := ensure(retained.stage == submissionStageZeroBytes && retained.writtenBytes == 0 && !retained.leased, "pre-write rejection requires its recorded writer exclusion"); err != nil {
			return none[submissionResult](), err
		}
		if deferred {
			if err := c.cancelDeferred(ctx, conn, observation, attemptKeyReceiptID(id), inputStateInProgress, "plain_limit_exceeded"); err != nil {
				return none[submissionResult](), err
			}
			c.clearPending()
			delete(c.operations, id)
			return none[submissionResult](), nil
		}
		operation, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (attemptRecord, error) {
			return c.recordState(ctx, conn, id, inputStateInProgress, inputStateReady)
		})
		if err != nil {
			return none[submissionResult](), err
		}
		retained = c.retained(id, "a zero-byte input retains its receipt")
		retained.operation = operation
		retained.attemptAvailable = false
		rejection.InputID = some(attemptKeyReceiptID(id))
		rejection.Delivery = some(attemptRecordReceiptOf(retained.operation).delivery())
	}
	c.clearPending()
	return some(rejection), nil
}

func (c *inputCoordinator) reselectZeroByte(ctx context.Context, conn *dbConn, id attemptKey, selection deliverySelection, deadline time.Time) error {
	retained := c.retained(id, "zero-byte receipt remains")
	if err := ensure(retained.stage == submissionStageZeroBytes && retained.writtenBytes == 0 && !retained.leased, "zero-byte selection requires its recorded writer exclusion"); err != nil {
		return err
	}
	switch record := retained.operation.(type) {
	case attemptRecordReceiptFree:
		record.record.deliveryMethod = some(selection.method)
		record.record.deliveryReason = selection.reason
		retained.operation = record
	case attemptRecordReceipt:
		if attemptRecordDeliveryMethod(record) != some(selection.method) || attemptRecordDeliveryReason(record) != selection.reason {
			expected := record.operation
			inputID := attemptKeyReceiptID(id)
			operation, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (inputOperation, error) {
				return inTransaction(conn, transactionImmediate, func() (inputOperation, error) {
					if err := c.validateLive(ctx, conn); err != nil {
						return inputOperation{}, err
					}
					current, err := getInput(ctx, conn, inputID)
					if err != nil {
						return inputOperation{}, err
					}
					if err := ensure(current == expected, "zero-byte selection receipt changed"); err != nil {
						return inputOperation{}, err
					}
					count, err := conn.execute("UPDATE member_input_operations SET delivery_method=?1,delivery_reason=?2,updated_at=?3 WHERE input_id=?4 AND state='in_progress'",
						selection.method.asStr(), selection.reason, wallStamp(c.services.clock), inputID)
					if err != nil {
						return inputOperation{}, err
					}
					if err := ensure(count == 1, "zero-byte selection requires its in-progress receipt"); err != nil {
						return inputOperation{}, err
					}
					return getInput(ctx, conn, inputID)
				})
			})
			if err != nil {
				return err
			}
			retained.operation = attemptRecordReceipt{operation: operation}
		}
	default:
		panic("invalid attemptRecord")
	}
	retained.bytes = selection.stages[0]
	retained.selection = &retainedSelectionComposer{method: selection.method, stages: selection.stages}
	return nil
}

func (c *inputCoordinator) reservePending(ctx context.Context, conn *dbConn, t *transport, observation *observationController, effective time.Time) (opt[inputControlReply], error) {
	seq, err := c.freshDrain(t, observation, effective)
	if err != nil {
		return none[inputControlReply](), err
	}
	outputSeq, fresh := seq.get()
	if !fresh {
		return none[inputControlReply](), nil
	}
	pending := c.pending
	var key attemptKey
	var operation attemptRecord
	switch delivery := pending.delivery.(type) {
	case pendingDeliveryPermission:
		if _, err := withDatabaseDeadline(ctx, conn, effective, func(ctx context.Context, conn *dbConn) (struct{}, error) {
			return inTransaction(conn, transactionDeferred, func() (struct{}, error) {
				if err := c.validateLive(ctx, conn); err != nil {
					return struct{}{}, err
				}
				if err := c.validatePermissionAssociation(ctx, conn, delivery.selection); err != nil {
					return struct{}{}, err
				}
				return struct{}{}, runtimeCheckDeadline(c.now(), effective)
			})
		}); err != nil {
			return none[inputControlReply](), err
		}
		key, operation = attemptKeyAnswer{}, receiptFreeRecordOf(none[reservationSelection]())
	case pendingDeliveryComposer:
		selection := reservationSelection{method: delivery.composer.method, reason: delivery.composer.reason}
		if _, preview := pending.request.kind.(attemptKindPreview); preview {
			key, operation = attemptKeySweepPreview{}, receiptFreeRecordOf(some(selection))
			break
		}
		reservation, err := withDatabaseDeadline(ctx, conn, effective, func(ctx context.Context, conn *dbConn) (inputReservation, error) {
			if err := c.validateLive(ctx, conn); err != nil {
				return inputReservation{}, err
			}
			return reserveInputWithProvenance(ctx, conn, c.run.owner(), pending.request.newInput(), newReservation{selection: some(selection), deferReason: none[string]()})
		})
		if err != nil {
			return none[inputControlReply](), err
		}
		if !reservation.inserted {
			reply := c.result(reservation.operation)
			c.clearPending()
			return some[inputControlReply](reply), nil
		}
		key, operation = attemptKeyReceipt{id: reservation.operation.inputID}, attemptRecordReceipt{operation: reservation.operation}
	default:
		panic("invalid pendingDelivery")
	}
	if err := ensure(attemptRecordState(operation) == inputStateInProgress, "ordinary input reservation is not writable"); err != nil {
		return none[inputControlReply](), err
	}
	var selection retainedSelection
	switch delivery := pending.delivery.(type) {
	case pendingDeliveryPermission:
		selection = &retainedSelectionPermission{selection: delivery.selection}
	case pendingDeliveryComposer:
		selection = &retainedSelectionComposer{method: delivery.composer.method, stages: delivery.composer.stages}
	default:
		panic("invalid pendingDelivery")
	}
	c.operations[key] = &retainedInput{operation: operation, bytes: pending.bytes, selection: selection, stage: submissionStageZeroBytes, outputSeq: outputSeq}
	pending.reservation = some(key)
	pending.phase = inputPhaseAwaitBeforeWrite
	return none[inputControlReply](), c.checkpoint(key, inputBoundaryReserved, effective)
}

func literalIndex(method deliveryMethod) int {
	if method == deliveryMethodFallback {
		return 1
	}
	return 0
}

// composerRendersLiteral reports, before a verified-composer submit key, that the composer
// renders exactly the written literal; every other stage, and a run whose profile reads no
// composer text, passes.
func (c *inputCoordinator) composerRendersLiteral(id attemptKey, t *transport) (bool, error) {
	if c.pending == nil || c.pending.composer() == nil {
		return true, nil
	}
	composer := c.pending.composer()
	selection, ok := c.retained(id, "pending receipt remains").selection.(*retainedSelectionComposer)
	if !ok {
		return false, runtimeError("ordinary writer selection changed")
	}
	index := literalIndex(composer.method)
	if selection.nextStage+1 != len(selection.stages) || selection.nextStage <= index || !c.verifiedComposer(composer.operation) {
		return true, nil
	}
	prefix := composer.operation.prefix()
	body, prefixed := bytes.CutPrefix(selection.stages[index], prefix)
	if !prefixed {
		return false, runtimeError("composer literal stage lacks its operation prefix")
	}
	if framed, open := bytes.CutPrefix(body, []byte(pasteOpen)); open {
		if inner, closed := bytes.CutSuffix(framed, []byte(pasteClose)); closed {
			body = inner
		}
	}
	if !utf8.Valid(body) {
		return false, runtimeError("composer literal is not UTF-8")
	}
	chunks := []string{}
	for _, chunk := range []string{string(prefix), string(body)} {
		if chunk != "" {
			chunks = append(chunks, chunk)
		}
	}
	snapshot := t.snapshot()
	text, read, err := c.composerReader.composerText(&snapshot)
	if err != nil {
		return false, err
	}
	return read && text.matches(chunks), nil
}

// preSubmitWindowOpen reports that a literal may still render: the composer is re-observed until
// the pre-submit window after the settle closes; the attempt deadline closes it earlier, and
// armPending then records the unmatched literal as literal_mismatch.
func (c *inputCoordinator) preSubmitWindowOpen() bool {
	if c.pending == nil || c.pending.composer() == nil {
		return false
	}
	until, settling := c.pending.composer().settleUntil.get()
	return settling && c.now().Before(until.Add(preSubmitWindow))
}

func (c *inputCoordinator) composerInterruption(id attemptKey, observation *observationController) (opt[stopCause], error) {
	pending := c.pending
	if pending == nil {
		panic("pending write remains")
	}
	composer := pending.composer()
	if composer == nil || !composer.started {
		return none[stopCause](), nil
	}
	selection, ok := c.retained(id, "pending receipt remains").selection.(*retainedSelectionComposer)
	if !ok {
		return none[stopCause](), runtimeError("ordinary writer selection changed")
	}
	betweenChunks := selection.nextStage == literalIndex(composer.method) && selection.offset > 0
	status := observation.status()
	if ordinaryBlockerOf(status) == some(ordinaryBlockerInteraction) {
		return some(interactionCause(status)), nil
	}
	remaining := max(pending.deadline.Sub(c.now()), 0)
	if (betweenChunks && remaining <= pasteCloseReserve) ||
		(selection.nextStage+1 == len(selection.stages) && c.verifiedComposer(composer.operation) && remaining < runtimeInputPostSubmitVerification+stageWriteMargin) {
		return some(stopCauseDeadlineExpired), nil
	}
	return none[stopCause](), nil
}

func (c *inputCoordinator) armPending(ctx context.Context, conn *dbConn, t *transport, observation *observationController, effective time.Time, terminalBookkeeping *bool) (opt[inputControlReply], error) {
	id, err := c.reservedAttempt()
	if err != nil {
		return none[inputControlReply](), err
	}
	progress, err := c.advanceStageGate(observation)
	if err != nil {
		return none[inputControlReply](), err
	}
	if progress == stageGateProgressWaiting {
		return none[inputControlReply](), nil
	}
	interruption, err := c.composerInterruption(id, observation)
	if err != nil {
		return none[inputControlReply](), err
	}
	if cause, interrupted := interruption.get(); interrupted {
		if cause == stopCauseDeadlineExpired {
			renders, err := c.composerRendersLiteral(id, t)
			if err != nil {
				return none[inputControlReply](), err
			}
			if !renders {
				cause = stopCauseLiteralMismatch
			}
		}
		*terminalBookkeeping = true
		return c.interruptComposer(ctx, conn, id, observation, effective, cause)
	}
	if composer := c.pending.composer(); composer != nil {
		if until, settling := composer.settleUntil.get(); settling && c.now().Before(until) {
			return none[inputControlReply](), nil
		}
	}
	seq, err := c.freshDrain(t, observation, effective)
	if err != nil {
		return none[inputControlReply](), err
	}
	renders, err := c.composerRendersLiteral(id, t)
	if err != nil {
		return none[inputControlReply](), err
	}
	if !renders {
		if c.preSubmitWindowOpen() {
			return none[inputControlReply](), nil
		}
		*terminalBookkeeping = true
		return c.interruptComposer(ctx, conn, id, observation, effective, stopCauseLiteralMismatch)
	}
	outputSeq, fresh := seq.get()
	if !fresh {
		return none[inputControlReply](), nil
	}
	retained := c.retained(id, "reserved input remains retained")
	retained.outputSeq = outputSeq
	retained.attemptAvailable = true
	c.pending.phase = inputPhaseArmed
	return none[inputControlReply](), c.checkpoint(id, inputBoundaryBeforeWrite, effective)
}

// interruptComposer stops a staged composer delivery; a deferred attempt publishes its partial
// result instead of replying.
func (c *inputCoordinator) interruptComposer(ctx context.Context, conn *dbConn, id attemptKey, observation *observationController, effective time.Time, cause stopCause) (opt[inputControlReply], error) {
	stop, needsClose, err := c.composerStop(id, cause)
	if err != nil {
		return none[inputControlReply](), err
	}
	if !stop.completePrefix && cause.isCheckStop() {
		stop.cause = stopCauseShortWrite
	}
	if needsClose {
		return none[inputControlReply](), c.preparePasteClose(id, stop)
	}
	return c.stopComposerStages(ctx, conn, id, observation, effective, stop)
}

func (c *inputCoordinator) preparePasteClose(id attemptKey, stop composerStop) error {
	pending := c.pending
	pending.composer().stop = some(stop)
	pending.bytes = []byte(pasteClose)
	pending.phase = inputPhaseClosing
	retained := c.retained(id, "stopped input remains")
	selection, ok := retained.selection.(*retainedSelectionComposer)
	if !ok {
		return runtimeError("paste cleanup requires ordinary input")
	}
	selection.closing = true
	retained.bytes = []byte(pasteClose)
	retained.attemptAvailable = false
	return nil
}

func (c *inputCoordinator) finishComposerStop(ctx context.Context, conn *dbConn, id attemptKey, observation *observationController, effective time.Time, stop composerStop) (opt[inputControlReply], error) {
	composer := c.pending.composer()
	failure := composer.stopFailure
	composer.stopFailure = nil
	reply, settled := c.stopComposerStages(ctx, conn, id, observation, effective, stop)
	switch {
	case failure != nil && settled == nil:
		return none[inputControlReply](), failure
	case failure != nil:
		return none[inputControlReply](), failure.withDiagnostic(cafleetErrorOf(settled).message())
	default:
		return reply, settled
	}
}

func (c *inputCoordinator) composerStop(id attemptKey, cause stopCause) (composerStop, bool, error) {
	retained, ok := c.operations[id]
	if !ok {
		return composerStop{}, false, runtimeError("stopped input is absent")
	}
	if c.pending == nil || c.pending.composer() == nil {
		return composerStop{}, false, runtimeError("stopped composer is absent")
	}
	composer := c.pending.composer()
	selection, isComposer := retained.selection.(*retainedSelectionComposer)
	if !isComposer {
		return composerStop{}, false, runtimeError("ordinary stop requires a composer selection")
	}
	index := literalIndex(composer.method)
	literal := selection.stages[index]
	written := 0
	switch {
	case selection.nextStage > index:
		written = len(literal)
	case selection.nextStage == index:
		written = selection.offset
	}
	prefix := len(composer.operation.prefix())
	bracketed := prefix <= len(literal) && bytes.HasPrefix(literal[prefix:], []byte(pasteOpen))
	completePrefix := utf8.Valid(literal[:written]) &&
		(!bracketed || written <= prefix ||
			(written >= prefix+len(pasteOpen) && written <= len(literal)-len(pasteClose)) || written == len(literal))
	needsClose := bracketed && written >= prefix+len(pasteOpen) && written < len(literal)
	return composerStop{cause: cause, completePrefix: completePrefix}, needsClose, nil
}

func (c *inputCoordinator) writePendingStage(ctx context.Context, conn *dbConn, t *transport, observation *observationController, pty ptyIo, effective time.Time, terminalBookkeeping *bool) (opt[inputControlReply], error) {
	id, err := c.reservedAttempt()
	if err != nil {
		return none[inputControlReply](), err
	}
	interruption, err := c.composerInterruption(id, observation)
	if err != nil {
		return none[inputControlReply](), err
	}
	if cause, interrupted := interruption.get(); interrupted {
		*terminalBookkeeping = true
		return c.interruptComposer(ctx, conn, id, observation, effective, cause)
	}
	offered := c.pending.bytes
	guard, guarded, err := c.acquireWriteGuard(ctx, conn, t, observation, effective)
	if err != nil || !guarded {
		return none[inputControlReply](), err
	}
	beforeStage, beforeCount := guard.retained.stage, guard.retained.writtenBytes
	write, writeErr := t.writeProgrammatic(pty, offered, guard, effective)
	attempted := guard.permit == guardPermitIoPossible || guard.permit == guardPermitResultRecorded
	guard.close()
	c.noteAnswerWritten(id)
	var after error
	if attempted {
		after = c.checkpoint(id, inputBoundaryAfterWrite, effective)
	}
	write, writeErr = combineManualProgress(write, writeErr, after)
	if writeErr == nil {
		writeErr = c.checkWriteEvidence(id, write, offered, beforeStage, beforeCount)
	}
	point, err := c.retainedCheckpoint(id)
	if err != nil {
		return none[inputControlReply](), err
	}
	composer := c.pending.composer()
	literalStage := composer != nil && composer.writtenStages == literalIndex(composer.method)
	if point.writtenBytes > 0 && composer != nil {
		composer.started = true
	}
	_, partial := write.(programmaticWritePartial)
	_, complete := write.(programmaticWriteComplete)
	_, pendingWrite := write.(programmaticWritePending)
	succeeded := writeErr == nil
	retained := c.retained(id, "actual stage remains retained")
	switch {
	case succeeded && partial && literalStage:
		c.pending.bytes = retained.bytes
		c.pending.phase = inputPhaseAwaitBeforeWrite
		return none[inputControlReply](), nil
	case succeeded && complete && retained.stage == submissionStagePartial:
		if composer != nil {
			composer.writtenStages++
			composer.settleUntil = some(c.now().Add(c.stageSettle()))
		}
		c.pending.bytes = retained.bytes
		c.pending.phase = inputPhaseAwaitBeforeWrite
		return none[inputControlReply](), nil
	case succeeded && complete && point.stage == submissionStageSubmitted && composer != nil && c.verifiedComposer(composer.operation):
		composer.verification = &postSubmitVerification{
			until:      c.now().Add(runtimeInputPostSubmitVerification),
			outputSeq:  t.snapshot().screenSeq,
			episodeSeq: observation.activeEpisodeSeq(),
		}
		c.pending.phase = inputPhaseVerifying
		return none[inputControlReply](), nil
	case succeeded && pendingWrite:
		c.pending.phase = inputPhaseAwaitBeforeWrite
		return none[inputControlReply](), nil
	}
	stage := retained.stage
	if composer != nil && stage != submissionStageZeroBytes && (!succeeded || !complete || stage != submissionStageSubmitted) {
		*terminalBookkeeping = true
		stop, needsClose, err := c.composerStop(id, stopCauseWriteFailed)
		if err != nil {
			return none[inputControlReply](), err
		}
		stop.completePrefix = false
		if writeErr != nil {
			composer.stopFailure = cafleetErrorOf(writeErr)
		} else {
			composer.stopFailure = nil
		}
		if needsClose {
			return none[inputControlReply](), c.preparePasteClose(id, stop)
		}
		return c.finishComposerStop(ctx, conn, id, observation, effective, stop)
	}
	*terminalBookkeeping = true
	if selection := c.pending.permission(); selection != nil {
		return c.finishAnswer(selection, id, write, writeErr, effective)
	}
	return c.completeWrite(ctx, conn, id, observation, effective, stage, writeErr)
}

// checkWriteEvidence requires the native result to agree with the guard's retained evidence.
func (c *inputCoordinator) checkWriteEvidence(id attemptKey, write programmaticWrite, offered []byte, beforeStage submissionStage, beforeCount uint64) error {
	point, err := c.retainedCheckpoint(id)
	if err != nil {
		return err
	}
	advanced := func(count int) (uint64, error) {
		if count < 0 {
			return 0, runtimeError("native input count exceeds range")
		}
		if uint64(count) > math.MaxUint64-beforeCount {
			return 0, runtimeError("native input count exhausted")
		}
		return beforeCount + uint64(count), nil
	}
	switch write := write.(type) {
	case programmaticWriteComplete:
		total, err := advanced(write.bytes)
		if err != nil {
			return err
		}
		return ensure(write.bytes == len(offered) && (point.stage == submissionStagePartial || point.stage == submissionStageSubmitted) &&
			point.writtenBytes == total, "complete native result conflicts with retained evidence")
	case programmaticWritePartial:
		total, err := advanced(write.bytes)
		if err != nil {
			return err
		}
		return ensure(write.bytes > 0 && write.bytes < len(offered) && point.stage == submissionStagePartial && point.writtenBytes == total,
			"partial native result conflicts with retained evidence")
	case programmaticWritePending:
		return ensure(point.stage == beforeStage && point.writtenBytes == beforeCount, "pending native result conflicts with retained evidence")
	default:
		panic("invalid programmaticWrite")
	}
}

// completeWrite records a finished prompt, exec or preview write by its stage and replies, or
// publishes a deferred attempt's result.
func (c *inputCoordinator) completeWrite(ctx context.Context, conn *dbConn, id attemptKey, observation *observationController, effective time.Time, stage submissionStage, writeErr error) (opt[inputControlReply], error) {
	persisted, persistErr := withDatabaseDeadline(ctx, conn, effective, func(ctx context.Context, conn *dbConn) (attemptRecord, error) {
		if err := c.validateLive(ctx, conn); err != nil {
			return nil, err
		}
		switch stage {
		case submissionStageSubmitted:
			return c.recordState(ctx, conn, id, inputStateInProgress, inputStateAccepted)
		case submissionStageZeroBytes:
			return c.recordState(ctx, conn, id, inputStateInProgress, inputStateReady)
		case submissionStagePartial, submissionStageUnknown:
			return c.recordUncertain(ctx, conn, id, stopCauseWriteFailed)
		default:
			panic("invalid submissionStage")
		}
	})
	if persistErr == nil {
		c.retained(id, "input retained").operation = persisted
	}
	c.pending.phase = inputPhaseBlocked
	switch {
	case writeErr != nil && persistErr != nil:
		return none[inputControlReply](), cafleetErrorOf(writeErr).withDiagnostic(cafleetErrorOf(persistErr).message())
	case writeErr != nil:
		return none[inputControlReply](), writeErr
	case persistErr != nil:
		return none[inputControlReply](), persistErr
	}
	if err := c.checkpoint(id, inputBoundaryTerminalState, effective); err != nil {
		return none[inputControlReply](), err
	}
	if c.pending.deferred() != nil {
		c.finishDeferred(observation, attemptRecordReceiptOf(persisted))
		c.clearPending()
		return none[inputControlReply](), nil
	}
	reply := c.attemptResult(id, persisted)
	c.clearPending()
	return some[inputControlReply](reply), nil
}

// noteAnswerWritten records that the pending answer's episode accepts no further answer once a
// write may have sent one of its bytes.
func (c *inputCoordinator) noteAnswerWritten(id attemptKey) {
	if c.pending == nil {
		return
	}
	selection := c.pending.permission()
	retained, ok := c.operations[id]
	if selection == nil || !ok {
		return
	}
	if retained.stage != submissionStageZeroBytes || retained.writtenBytes > 0 {
		c.answeredInteractions[selection.request.InteractionID] = struct{}{}
	}
}

// answerStopReason is why a written answer lost its authority: positive manual input or failed
// manual-input accounting since the answer was bound, then a positive short stage write, and
// otherwise a failed write.
func (c *inputCoordinator) answerStopReason(selection *permissionSelection, shortWrite bool) string {
	binding := &selection.binding
	switch {
	case c.manualAccountingFailed || c.manualGeneration != binding.manualGeneration || c.pointerGeneration != binding.pointerGeneration:
		return "manual_input"
	case shortWrite:
		return stopCauseShortWrite.asStr()
	default:
		return stopCauseWriteFailed.asStr()
	}
}

func (c *inputCoordinator) answerReply(selection *permissionSelection, id attemptKey, status submissionStatus, reason opt[string]) (inputControlReply, error) {
	point, err := c.retainedCheckpoint(id)
	if err != nil {
		return nil, err
	}
	return c.permissionReply(selection.request, submissionResult{
		Status:    status,
		InputID:   none[int64](),
		Stage:     point.stage,
		OutputSeq: some(point.outputSeq),
		Reason:    reason,
		Delivery:  none[deliveryProjection](),
	}), nil
}

func (c *inputCoordinator) endAnswer() {
	c.clearPending()
	delete(c.operations, attemptKeyAnswer{})
}

// finishAnswer replies to an answer whose stages ended: accepted once its final stage is
// completely written and uncertain after any other written byte. A failure before any byte stays
// an error and leaves the episode answerable.
func (c *inputCoordinator) finishAnswer(selection *permissionSelection, id attemptKey, write programmaticWrite, writeErr error, effective time.Time) (opt[inputControlReply], error) {
	c.pending.phase = inputPhaseBlocked
	point, err := c.retainedCheckpoint(id)
	if err != nil {
		return none[inputControlReply](), err
	}
	var reply inputControlReply
	switch {
	case writeErr == nil && point.stage == submissionStageSubmitted:
		if reply, err = c.answerReply(selection, id, submissionStatusAccepted, none[string]()); err != nil {
			return none[inputControlReply](), err
		}
	case writeErr == nil && point.stage == submissionStageZeroBytes:
		panic("a completed answer write records its written bytes")
	case writeErr != nil && point.stage == submissionStageZeroBytes:
		return none[inputControlReply](), writeErr
	default:
		_, shortWrite := write.(programmaticWritePartial)
		shortWrite = shortWrite && writeErr == nil
		if writeErr != nil {
			c.logAnswerFailure(selection, cafleetErrorOf(writeErr))
		}
		if reply, err = c.answerReply(selection, id, submissionStatusUncertain, some(c.answerStopReason(selection, shortWrite))); err != nil {
			return none[inputControlReply](), err
		}
	}
	if err := c.checkpoint(id, inputBoundaryTerminalState, effective); err != nil {
		return none[inputControlReply](), err
	}
	c.endAnswer()
	return some[inputControlReply](reply), nil
}

func (c *inputCoordinator) logAnswerFailure(selection *permissionSelection, cause *cafleetError) {
	c.recordDiagnostic(runnerDiagnosticSourcePermissionAnswer, fmt.Sprintf("permission answer for interaction %d is uncertain: %s", selection.request.InteractionID, cause.message()))
}

// answerFailure turns a failure of the pending answer after a written byte into its uncertain
// reply; any other failure returns unchanged as the second result.
func (c *inputCoordinator) answerFailure(source *cafleetError) (inputControlReply, *cafleetError) {
	pending := c.pending
	if pending == nil {
		return nil, source
	}
	selection := pending.permission()
	if selection == nil {
		return nil, source
	}
	pending.phase = inputPhaseBlocked
	id, reserved := pending.attempt().get()
	if !reserved {
		return nil, source
	}
	retained := c.retained(id, "pending answer record retained")
	if retained.leased {
		panic("failed attempt has released its guard")
	}
	retained.attemptAvailable = false
	if retained.stage == submissionStageZeroBytes && retained.writtenBytes == 0 {
		return nil, source
	}
	c.logAnswerFailure(selection, source)
	reply, err := c.answerReply(selection, id, submissionStatusUncertain, some(c.answerStopReason(selection, false)))
	if err != nil {
		return nil, source.withDiagnostic(cafleetErrorOf(err).message())
	}
	c.endAnswer()
	return reply, nil
}

// activeDeferred is the started deferred attempt's input, its stage and whether it may have
// written a child byte; false when no deferred attempt is started.
func (c *inputCoordinator) activeDeferred() (int64, submissionStage, bool, bool) {
	if c.pending == nil || c.pending.deferred() == nil {
		return 0, 0, false, false
	}
	key, set := c.pending.attempt().get()
	if !set {
		panic("a deferred attempt has its receipt")
	}
	stage := c.retained(key, "a deferred attempt retains its receipt").stage
	written := c.pending.composer().writtenStages > 0
	return attemptKeyReceiptID(key), stage, stage != submissionStageZeroBytes || written, true
}

// recordDeferredUncertain records a still-in_progress deferred receipt as uncertain after a
// possible write and returns the receipt as stored.
func (c *inputCoordinator) recordDeferredUncertain(ctx context.Context, conn *dbConn, id int64, cause stopCause, deadline time.Time) (inputOperation, error) {
	current, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (inputOperation, error) {
		return getInput(ctx, conn, id)
	})
	if err != nil {
		return inputOperation{}, err
	}
	operation := current
	if current.state == inputStateInProgress {
		if operation, err = withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (inputOperation, error) {
			return recordInputUncertain(ctx, conn, c.run.owner(), id, cause)
		}); err != nil {
			return inputOperation{}, err
		}
	}
	retained := c.retained(attemptKeyReceipt{id: id}, "a deferred attempt retains its receipt")
	retained.operation = attemptRecordReceipt{operation: operation}
	retained.attemptAvailable = false
	return operation, nil
}

// retireFailedDeferred ends a deferred attempt that failed after a possible write: its receipt
// becomes uncertain, its result is published once and the queue waits for reconciliation.
// Without a durable uncertain receipt, recovery publishes the result instead.
func (c *inputCoordinator) retireFailedDeferred(ctx context.Context, conn *dbConn, observation *observationController) {
	id, stage, written, active := c.activeDeferred()
	if !active || !written {
		return
	}
	c.clearPending()
	operation, err := c.recordDeferredUncertain(ctx, conn, id, stopCauseWriteFailed, c.now().Add(deferredFinalization))
	switch {
	case err != nil:
		c.recordPendingRecovery(id, cafleetErrorOf(err))
	case operation.state == inputStateUncertain:
		publishResult(observation, id, stage, operation.stopCause)
	default:
		c.finishDeferred(observation, operation)
	}
}

func (c *inputCoordinator) stopComposerStages(ctx context.Context, conn *dbConn, id attemptKey, observation *observationController, effective time.Time, stop composerStop) (opt[inputControlReply], error) {
	pending := c.pending
	pending.phase = inputPhaseBlocked
	deferred := pending.composer().deferred != nil
	bookkeeping := effective
	if !effective.After(c.now()) {
		bookkeeping = c.now().Add(deferredFinalization)
	}
	operation, err := withDatabaseDeadline(ctx, conn, bookkeeping, func(ctx context.Context, conn *dbConn) (attemptRecord, error) {
		if err := c.validateLiveRun(ctx, conn); err != nil {
			return nil, err
		}
		return c.recordUncertain(ctx, conn, id, stop.cause)
	})
	if err != nil {
		return none[inputControlReply](), err
	}
	retained := c.retained(id, "stopped receipt remains")
	retained.operation = operation
	retained.attemptAvailable = false
	stage := retained.stage
	if err := c.checkpoint(id, inputBoundaryTerminalState, bookkeeping); err != nil {
		return none[inputControlReply](), err
	}
	c.publishOutcome(observation, id, stage, attemptRecordStopCause(operation))
	reply := c.attemptResult(id, operation)
	c.clearPending()
	if deferred {
		return none[inputControlReply](), nil
	}
	return some[inputControlReply](reply), nil
}

func (c *inputCoordinator) writeCloseStage(ctx context.Context, conn *dbConn, t *transport, pty ptyIo, observation *observationController, effective time.Time) (opt[inputControlReply], error) {
	id, err := c.reservedAttempt()
	if err != nil {
		return none[inputControlReply](), err
	}
	held, err := c.holdComposerPane()
	if err != nil || !held {
		return none[inputControlReply](), err
	}
	if _, err := withDatabaseDeadline(ctx, conn, effective, func(ctx context.Context, conn *dbConn) (struct{}, error) {
		if err := c.validateLiveRun(ctx, conn); err != nil {
			return struct{}{}, err
		}
		if err := c.lock.validate(ctx, conn, c.run.owner()); err != nil {
			return struct{}{}, err
		}
		current, err := c.currentInput(ctx, conn, id)
		if err != nil {
			return struct{}{}, err
		}
		return struct{}{}, ensure(current == c.retained(id, "closing input remains").operation, "paste cleanup receipt changed")
	}); err != nil {
		return none[inputControlReply](), err
	}
	retained := c.retained(id, "closing input remains")
	if err := ensure(!retained.leased, "paste cleanup requires exclusive input authority"); err != nil {
		return none[inputControlReply](), err
	}
	retained.leased = true
	guard := &inputGuard{retained: retained, deadline: effective, permit: guardPermitFresh, clock: c.services.clock}
	written, writeErr := t.writeProgrammatic(pty, []byte(pasteClose), guard, effective)
	guard.close()
	after := c.checkpoint(id, inputBoundaryAfterWrite, effective)
	written, writeErr = combineManualProgress(written, writeErr, after)
	if _, pending := written.(programmaticWritePending); pending && writeErr == nil {
		return none[inputControlReply](), nil
	}
	composer := c.pending.composer()
	stop, set := composer.stop.get()
	if !set {
		panic("closing input has its stopped outcome")
	}
	_, complete := written.(programmaticWriteComplete)
	complete = complete && writeErr == nil
	if !complete {
		stop.cause = stopCauseWriteFailed
		stop.completePrefix = false
	}
	switch {
	case writeErr != nil:
		if composer.stopFailure != nil {
			composer.stopFailure = composer.stopFailure.withDiagnostic(cafleetErrorOf(writeErr).message())
		} else {
			composer.stopFailure = cafleetErrorOf(writeErr)
		}
	case !complete && composer.stopFailure != nil:
		composer.stopFailure = composer.stopFailure.withDiagnostic("paste closing marker was not completely written")
	}
	return c.finishComposerStop(ctx, conn, id, observation, effective, stop)
}

func (c *inputCoordinator) verifySubmit(ctx context.Context, conn *dbConn, t *transport, observation *observationController, effective time.Time) (opt[inputControlReply], error) {
	id, err := c.reservedAttempt()
	if err != nil {
		return none[inputControlReply](), err
	}
	verification := c.pending.composer().verification
	if verification == nil {
		panic("active verification is required")
	}
	fresh := false
	if proof := t.takeInputDrain(); proof != nil {
		if err := t.validateOrdinary(proof, effective); err != nil {
			return none[inputControlReply](), err
		}
		fresh = proof.outputSeq > verification.outputSeq
	}
	episode, active := observation.activeEpisodeSeq().get()
	before, earlier := verification.episodeSeq.get()
	newInteraction := active && (!earlier || episode > before) &&
		observation.status().eligibility != inputEligibility(inputEligibilityHold{reason: gateReasonCandidate})
	if fresh && (composerEligible(observation) || newInteraction) {
		operation, err := withDatabaseDeadline(ctx, conn, effective, func(ctx context.Context, conn *dbConn) (attemptRecord, error) {
			if err := c.validateLiveRun(ctx, conn); err != nil {
				return nil, err
			}
			return c.recordState(ctx, conn, id, inputStateInProgress, inputStateAccepted)
		})
		if err != nil {
			return none[inputControlReply](), err
		}
		c.retained(id, "verified input remains").operation = operation
		if c.pending.deferred() != nil {
			c.finishDeferred(observation, attemptRecordReceiptOf(operation))
			c.clearPending()
			return none[inputControlReply](), nil
		}
		reply := c.attemptResult(id, operation)
		c.clearPending()
		return some[inputControlReply](reply), nil
	}
	if !c.now().Before(verification.until) {
		return c.stopComposerStages(ctx, conn, id, observation, effective, composerStop{cause: stopCauseSubmitUnconfirmed, completePrefix: true})
	}
	return none[inputControlReply](), nil
}

// pendingAttemptFailed ends the in-flight immediate attempt whose completion failed before the
// runner replies: a written answer replies uncertain, a written prompt or exec records
// uncertain/write_failed, and every other failure returns to the client.
func (c *inputCoordinator) pendingAttemptFailed(ctx context.Context, conn *dbConn, observation *observationController, failure *inputFailure) (inputControlReply, *inputFailure) {
	reply, source := c.answerFailure(failure.source)
	if source == nil {
		return reply, nil
	}
	source = c.recordImmediateWriteFailure(ctx, conn, observation, c.now(), source)
	return nil, &inputFailure{source: source, context: failure.context}
}

func (c *inputCoordinator) recordImmediateWriteFailure(ctx context.Context, conn *dbConn, observation *observationController, deadline time.Time, source *cafleetError) *cafleetError {
	pending := c.pending
	if pending == nil {
		return source
	}
	composer := pending.composer()
	if composer == nil || composer.deferred != nil {
		return source
	}
	id, reserved := pending.attempt().get()
	if !reserved {
		return source
	}
	retained := c.retained(id, "pending composer receipt retained")
	written := retained.stage != submissionStageZeroBytes || retained.writtenBytes > 0 || composer.writtenStages > 0
	if !written || attemptRecordState(retained.operation) != inputStateInProgress {
		return source
	}
	pending.phase = inputPhaseBlocked
	stage := retained.stage
	bookkeeping := c.now().Add(deferredFinalization)
	if effective, err := c.effectiveDeadline(ctx, deadline); err == nil && effective.After(c.now()) {
		bookkeeping = effective
	}
	persisted, err := withDatabaseDeadline(ctx, conn, bookkeeping, func(ctx context.Context, conn *dbConn) (attemptRecord, error) {
		if err := c.validateLiveRun(ctx, conn); err != nil {
			return nil, err
		}
		return c.recordUncertain(ctx, conn, id, stopCauseWriteFailed)
	})
	retained = c.retained(id, "pending composer receipt retained")
	retained.attemptAvailable = false
	if err != nil {
		return source.withDiagnostic(cafleetErrorOf(err).message())
	}
	retained.operation = persisted
	c.publishOutcome(observation, id, stage, some(stopCauseWriteFailed))
	return source
}

// finishPendingAttempt retires the pending immediate attempt; a started deferred attempt stays
// for fence, which settles it by its written bytes.
func (c *inputCoordinator) finishPendingAttempt() {
	if _, _, _, active := c.activeDeferred(); active {
		return
	}
	if c.pending != nil {
		if key, set := c.pending.attempt().get(); set {
			retained := c.retained(key, "pending input reservation remains retained")
			if retained.leased {
				panic("completed input has no active guard lease")
			}
			retained.attemptAvailable = false
		}
	}
	delete(c.operations, attemptKeySweepPreview{})
	delete(c.operations, attemptKeyAnswer{})
	c.clearPending()
}

func newInputCoordinator(ctx context.Context, conn *dbConn, run runRecord, lock *runLock, services inputServices, deadline time.Time) (*inputCoordinator, error) {
	current, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (runRecord, error) {
		return inTransaction(conn, transactionDeferred, func() (runRecord, error) {
			return validateCoordinatorRun(ctx, conn, run, lock, services.clock, deadline)
		})
	})
	if err != nil {
		return nil, err
	}
	if conn.path() == "" {
		return nil, runtimeError("input coordinator database has no file path")
	}
	member, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (memberRecord, error) {
		found, err := getMember(ctx, conn, current.memberID, current.fleetID)
		if err != nil {
			return memberRecord{}, err
		}
		member, present := found.get()
		if !present {
			return memberRecord{}, runtimeError("input coordinator member is absent")
		}
		return member, ensure(member.kind == memberKindMember, "input coordinator requires an ordinary member")
	})
	if err != nil {
		return nil, err
	}
	backend, known := codingAgentCodingAgent(current.backend)
	if !known {
		return nil, runtimeError("unknown input backend")
	}
	return &inputCoordinator{
		composerReader:       newDetector(services.catalog, backend, none[string]()),
		placement:            member.placement,
		operations:           map[attemptKey]*retainedInput{},
		answeredInteractions: map[int64]struct{}{},
		run:                  current,
		backend:              backend,
		lock:                 lock,
		services:             services,
		observationBlocker:   none[string](),
		databaseURL:          "sqlite:///" + conn.path(),
		stallBound:           stallBound,
		diagnostics:          newRunnerDiagnostics(),
	}, nil
}

func validateCoordinatorRun(ctx context.Context, conn *dbConn, run runRecord, lock *runLock, c clock, deadline time.Time) (runRecord, error) {
	current, err := validateOwner(ctx, conn, run.owner())
	if err != nil {
		return runRecord{}, err
	}
	if err := ensure(jsonEqual(stableRunIdentity(current), stableRunIdentity(run)), "input coordinator run identity changed"); err != nil {
		return runRecord{}, err
	}
	if err := ensure(current.ownerKind == ownerKindRunner && current.status == runStatusRunning, "input coordinator requires a running runner owner"); err != nil {
		return runRecord{}, err
	}
	process, err := processIdentityCurrent()
	if err != nil {
		return runRecord{}, err
	}
	if err := ensure(current.ownerPID == process.pid && current.ownerStartIdentity == process.startIdentity &&
		current.runnerPID == some(process.pid) && current.runnerStartIdentity == some(process.startIdentity),
		"input coordinator runner generation mismatch"); err != nil {
		return runRecord{}, err
	}
	if err := lock.validate(ctx, conn, current.owner()); err != nil {
		return runRecord{}, err
	}
	return current, runtimeCheckDeadline(c.now(), deadline)
}

// shareDiagnostics records input diagnostics into the runner's shared buffer, which the event
// writer flushes.
func (c *inputCoordinator) shareDiagnostics(diagnostics runnerDiagnostics) {
	c.diagnostics = diagnostics
}

func (c *inputCoordinator) recordDiagnostic(source runnerDiagnosticSource, message string) {
	c.diagnostics.record(source, message, wallStamp(c.services.clock))
}

func (c *inputCoordinator) recordPendingRecovery(inputID int64, cause *cafleetError) {
	c.recordDiagnostic(runnerDiagnosticSourceInputResult, fmt.Sprintf("input %d result is pending recovery: %s", inputID, cause.message()))
}

// progressWithManual runs a transport step and accounts the manual and pointer input it read; a
// failed accounting fences programmatic input.
func progressWithManual[T any](c *inputCoordinator, t *transport, step func(*transport) (T, error)) (T, *inputFailure) {
	before, beforePointer := t.humanGeneration, t.pointerGeneration
	progress, progressErr := step(t)
	c.outputSeq = t.snapshot().screenSeq
	c.bracketedPaste = t.bracketedPaste()
	c.observationBlocker = t.drainBlocker
	var accounting error
	if t.humanGeneration < before {
		accounting = runtimeError("manual input generation decreased")
	} else {
		accounting = c.observeManual(t.humanGeneration - before)
	}
	var pointerAccounting error
	switch {
	case t.pointerGeneration < beforePointer:
		pointerAccounting = runtimeError("pointer input generation decreased")
	default:
		count := t.pointerGeneration - beforePointer
		if count > 0 {
			c.currentRequest = nil
		}
		if count > math.MaxUint64-c.pointerGeneration {
			pointerAccounting = runtimeError("pointer input generation exhausted")
		} else {
			c.pointerGeneration += count
		}
	}
	_, accounting = combineManualProgress(struct{}{}, accounting, pointerAccounting)
	if accounting != nil {
		c.manualAccountingFailed = true
	}
	result, err := combineManualProgress(progress, progressErr, accounting)
	if err != nil {
		return result, c.failure(cafleetErrorOf(err))
	}
	return result, nil
}

func (c *inputCoordinator) observeManual(count uint64) error {
	if count > 0 {
		c.currentRequest = nil
	}
	if count > math.MaxUint64-c.manualGeneration {
		c.manualAccountingFailed = true
		return runtimeError("manual input generation exhausted")
	}
	c.manualGeneration += count
	return nil
}
