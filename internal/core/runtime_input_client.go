package core

import (
	"context"
	"slices"
	"strconv"
	"strings"
	"time"
)

type preparedSubmission struct {
	request    submitInput
	expiration inputExpiration
	deadline   time.Time
}

func newPreparedSubmission(request submitInput, deadline time.Time, c clock) (preparedSubmission, error) {
	expiration, err := inputExpirationCapture(deadline, c)
	if err != nil {
		return preparedSubmission{}, err
	}
	return preparedSubmission{request: request, expiration: expiration, deadline: deadline}, nil
}

type inputFailure struct {
	source  *cafleetError
	context inputContext
}

// asError is the failure's source with one diagnostic naming its known identifiers.
func (f *inputFailure) asError() *cafleetError {
	var fields []string
	for _, field := range []struct {
		name  string
		value opt[int64]
	}{
		{"run_id", f.context.runID},
		{"interaction_id", f.context.interactionID},
		{"input_id", f.context.inputID},
	} {
		if value, set := field.value.get(); set {
			fields = append(fields, field.name+"="+strconv.FormatInt(value, 10))
		}
	}
	if key, set := f.context.operationKey.get(); set {
		fields = append(fields, "operation_key="+string(jsonValue(key)))
	}
	if len(fields) == 0 {
		return f.source
	}
	return f.source.withDiagnostic(strings.Join(fields, " "))
}

type inputContext struct {
	runID         opt[int64]
	interactionID opt[int64]
	inputID       opt[int64]
	operationKey  opt[string]
}

func (c inputContext) failure(source *cafleetError) *inputFailure {
	return &inputFailure{source: source, context: c}
}

// failureOf wraps err, a cafleet error, with the identifiers context knows.
func (c inputContext) failureOf(err error) *inputFailure {
	return c.failure(cafleetErrorOf(err))
}

type inputCommandHost interface {
	clock() clock
	lock(s settings, p placement, d deadline) (*paneLock, bool, error)
	health(run runRecord, deadline time.Time) (healthSnapshot, error)
	exchange(run runRecord, request inputControlRequest, deadline time.Time) (inputControlReply, error)
}

type nativeInputHost struct{}

func (nativeInputHost) clock() clock {
	return systemClock{}
}

func (nativeInputHost) lock(s settings, p placement, d deadline) (*paneLock, bool, error) {
	return paneLockAcquireWithin(s.databaseURL, p, d)
}

func (h nativeInputHost) health(run runRecord, deadline time.Time) (healthSnapshot, error) {
	return probeHealth(run, deadline, h.clock())
}

func (h nativeInputHost) exchange(run runRecord, request inputControlRequest, deadline time.Time) (inputControlReply, error) {
	paths, err := runtimePathsForRun(run.databaseIdentityJSON, run.runID)
	if err != nil {
		return nil, err
	}
	client, err := controlClientConnect(paths.controlSocket, deadline, h.clock())
	if err != nil {
		return nil, err
	}
	reply, err := client.exchangeInput(request, deadline, h.clock())
	return reply, mergeCleanup(err, client.close())
}

// bracketHealth probes the runner's health between two revalidations of the durable target.
func bracketHealth(host inputCommandHost, run runRecord, deadline time.Time, validate func() error) error {
	if err := validate(); err != nil {
		return err
	}
	if _, err := host.health(run, deadline); err != nil {
		return err
	}
	return validate()
}

func controlRequest(run runRecord, expiration inputExpiration, operation inputControlOperation) inputControlRequest {
	return inputControlRequest{
		Identity:        controlIdentityFromRun(run),
		ExpiresAtMonoNS: expiration.monotonicNs(),
		Operation:       operation,
	}
}

func submissionContext(runID int64, request submitInput) inputContext {
	return inputContext{runID: some(runID), operationKey: some(request.OperationKey)}
}

func validateSubmissionRequest(runID int64, request submitInput) error {
	if err := validateOperationKey(request.OperationKey); err != nil {
		return err
	}
	return usage(
		runID > 0 && validInput(request.Literal),
		"ordinary submission requires a positive run and one nonblank line without control characters",
	)
}

// inputIDOf is the identity of a retained input, if any.
func inputIDOf(retained opt[inputOperation]) opt[int64] {
	if input, set := retained.get(); set {
		return some(input.inputID)
	}
	return none[int64]()
}

func findSubmission(ctx context.Context, conn *dbConn, identity permissionReceiptIdentity, request submitInput, d deadline, c clock) (opt[inputOperation], *inputFailure) {
	scope := submissionContext(identity.runID, request)
	retained, err := func() (opt[inputOperation], error) {
		if err := validateSubmissionRequest(identity.runID, request); err != nil {
			return none[inputOperation](), err
		}
		return withDatabaseBudget(ctx, conn, d, func(ctx context.Context, conn *dbConn) (opt[inputOperation], error) {
			return inTransaction(conn, transactionDeferred, func() (opt[inputOperation], error) {
				run, err := selectedRun(ctx, conn, identity.runID)
				if err != nil {
					return none[inputOperation](), err
				}
				if err := ensure(jsonEqual(run.databaseIdentityJSON, identity.databaseIdentityJSON), "submission database identity mismatch"); err != nil {
					return none[inputOperation](), err
				}
				retained, err := findInput(ctx, conn, identity.runID, request.OperationKey)
				if err != nil {
					return none[inputOperation](), err
				}
				if input, set := retained.get(); set {
					scope.inputID = some(input.inputID)
					digest := inputPayloadDigest(identity.runID, request)
					if err := ensure(input.kind == request.InputKind && input.payloadDigest == digest, "conflicting input operation key reuse"); err != nil {
						return none[inputOperation](), err
					}
				}
				if _, err := d.remainingAt(c.now()); err != nil {
					return none[inputOperation](), err
				}
				return retained, nil
			})
		})
	}()
	if err != nil {
		return none[inputOperation](), scope.failureOf(err)
	}
	return retained, nil
}

type submissionOutcome struct {
	status submissionStatus
	stage  submissionStage
	reason opt[string]
}

// durableOutcome is the outcome a durable input already determines; a ready input that was not
// deferred needs live revalidation.
func durableOutcome(input inputOperation) opt[submissionOutcome] {
	disposition, disposed := input.disposition.get()
	submitted := disposed && disposition == inputDispositionSubmitted
	switch input.state {
	case inputStateReady:
		if input.deferredAt.set {
			return some(submissionOutcome{status: submissionStatusQueued, stage: submissionStageZeroBytes, reason: input.deferReason})
		}
		return none[submissionOutcome]()
	case inputStateAccepted:
		return some(submissionOutcome{status: submissionStatusAccepted, stage: submissionStageSubmitted})
	case inputStateSettled:
		if submitted {
			return some(submissionOutcome{status: submissionStatusAccepted, stage: submissionStageSubmitted})
		}
		return some(submissionOutcome{status: submissionStatusRejected, stage: submissionStageZeroBytes, reason: some("input operation key is consumed")})
	case inputStateInProgress, inputStateUncertain:
		return some(submissionOutcome{status: submissionStatusUncertain, stage: submissionStageUnknown, reason: uncertainReason(input)})
	default:
		panic("invalid inputState")
	}
}

func uncertainReason(input inputOperation) opt[string] {
	if cause, set := input.stopCause.get(); set {
		return some(cause.asStr())
	}
	return none[string]()
}

func historicalSubmission(retained opt[inputOperation]) opt[submissionResult] {
	input, set := retained.get()
	if !set {
		return none[submissionResult]()
	}
	outcome, determined := durableOutcome(input).get()
	if !determined {
		return none[submissionResult]()
	}
	return some(submissionResult{
		Status:   outcome.status,
		InputID:  some(input.inputID),
		Stage:    outcome.stage,
		Reason:   outcome.reason,
		Delivery: some(input.delivery()),
	})
}

func submissionDeadline(ctx context.Context, d deadline, c clock) (deadline, error) {
	bounded := deadlineUntil(boundedByInherited(ctx, d.instant()))
	if _, err := bounded.remainingAt(c.now()); err != nil {
		return deadline{}, err
	}
	return bounded, nil
}

func sameEventRecord(a, b runtimeEventRecord) bool {
	return a.eventID == b.eventID && a.runID == b.runID && a.eventSeq == b.eventSeq &&
		a.interactionID == b.interactionID && a.kind == b.kind && a.observedAt == b.observedAt &&
		jsonEqual(a.payloadJSON, b.payloadJSON) && a.notificationState == b.notificationState &&
		a.messageID == b.messageID && a.archiveReason == b.archiveReason
}

func sameAssociation(view, original runtimeEventView) bool {
	type association struct {
		interactionID  int64
		runID          int64
		episodeSeq     int64
		classification interactionClass
	}
	interaction := func(view runtimeEventView) opt[association] {
		record, set := view.interaction.get()
		if !set {
			return none[association]()
		}
		return some(association{record.interactionID, record.runID, record.episodeSeq, record.classification})
	}
	return sameEventRecord(view.event, original.event) && interaction(view) == interaction(original)
}

func permissionPlacement(ctx context.Context, conn *dbConn, run runRecord) (placement, error) {
	member, err := getMember(ctx, conn, run.memberID, run.fleetID)
	if err != nil {
		return placement{}, err
	}
	record, found := member.get()
	if !found {
		return placement{}, runtimeError("permission member placement unavailable")
	}
	return record.placement, nil
}

// replyMessages are the identity, contradiction, missing-durable and unmatched errors of
// validateReplyAgainstDurable, in that order.
type replyMessages = [4]string

func validateReplyAgainstDurable(reply submissionResult, durable opt[inputOperation], messages replyMessages) error {
	identity, contradiction, missing, unmatched := messages[0], messages[1], messages[2], messages[3]
	id, replied := reply.InputID.get()
	input, recorded := durable.get()
	switch {
	case replied && recorded:
		if err := ensure(id == input.inputID, identity); err != nil {
			return err
		}
		disposition, disposed := input.disposition.get()
		submitted := disposed && disposition == inputDispositionSubmitted
		var consistent bool
		switch reply.Status {
		case submissionStatusAccepted:
			consistent = reply.Stage == submissionStageSubmitted &&
				(input.state == inputStateAccepted || input.state == inputStateSettled && submitted)
		case submissionStatusQueued:
			consistent = reply.Stage == submissionStageZeroBytes && input.state == inputStateReady && input.deferredAt.set
		case submissionStatusUncertain:
			consistent = (input.state == inputStateInProgress || input.state == inputStateUncertain) &&
				reply.Reason == uncertainReason(input)
		case submissionStatusHeld, submissionStatusRejected:
			consistent = reply.Stage == submissionStageZeroBytes &&
				(input.state == inputStateReady || input.state == inputStateSettled && !submitted)
		default:
			panic("invalid submissionStatus")
		}
		return ensure(consistent && reply.Delivery == some(input.delivery()), contradiction)
	case !replied && !recorded:
		return ensure(
			(reply.Status == submissionStatusHeld || reply.Status == submissionStatusRejected) &&
				reply.Stage == submissionStageZeroBytes,
			missing,
		)
	default:
		return runtimeError(unmatched)
	}
}

func validateSubmissionTarget(ctx context.Context, conn *dbConn, s settings, expected runRecord, p placement) error {
	if err := validateConfiguredDatabase(conn, s, "file-backed input database required", "submission connection and configured database differ"); err != nil {
		return err
	}
	current, err := revalidateRunSnapshot(ctx, conn, expected, p, snapshotPolicyLive, snapshotMessages{
		runChanged:       "submission run identity changed",
		placementChanged: "submission member placement changed",
	})
	if err != nil {
		return err
	}
	_, err = validateRunPaths(ctx, conn, current)
	return err
}

func submitWrappedInput(ctx context.Context, conn *dbConn, s settings, expectedRun runRecord, expectedPlacement placement, request submitInput, d deadline) (submissionResult, *inputFailure) {
	return submitWrappedInputWithHost(ctx, conn, s, expectedRun, expectedPlacement, request, d, nativeInputHost{})
}

func submitWrappedInputWithHost(ctx context.Context, conn *dbConn, s settings, expectedRun runRecord, expectedPlacement placement, request submitInput, d deadline, host inputCommandHost) (submissionResult, *inputFailure) {
	history, prepared, failure := prepareSubmissionWithHost(ctx, conn, s, expectedRun, expectedPlacement, request, d, host)
	if failure != nil {
		return submissionResult{}, failure
	}
	if result, historical := history.get(); historical {
		return result, nil
	}
	defer prepared.held.close()
	return submitWrappedInputUnderPaneLockWithHost(ctx, conn, s, expectedRun, expectedPlacement, prepared.held, prepared.attempt, prepared.deadline, host)
}

// lockedSubmission is a submission prepared under its held pane lock.
type lockedSubmission struct {
	attempt  preparedSubmission
	held     *paneLock
	deadline deadline
}

// prepareSubmissionWithHost returns the outcome a durable input already determines, or else the
// attempt prepared under the member's pane lock.
func prepareSubmissionWithHost(ctx context.Context, conn *dbConn, s settings, expectedRun runRecord, expectedPlacement placement, request submitInput, d deadline, host inputCommandHost) (opt[submissionResult], lockedSubmission, *inputFailure) {
	c := host.clock()
	identity := permissionReceiptIdentityForRun(expectedRun)
	retained, failure := findSubmission(ctx, conn, identity, request, d, c)
	if failure != nil {
		return none[submissionResult](), lockedSubmission{}, failure
	}
	if history := historicalSubmission(retained); history.set {
		return history, lockedSubmission{}, nil
	}
	scope := submissionContext(expectedRun.runID, request)
	scope.inputID = inputIDOf(retained)
	locked, err := func() (lockedSubmission, error) {
		bounded, err := submissionDeadline(ctx, d, c)
		if err != nil {
			return lockedSubmission{}, err
		}
		if _, err := withDatabaseBudget(ctx, conn, bounded, func(ctx context.Context, conn *dbConn) (struct{}, error) {
			return struct{}{}, validateSubmissionTarget(ctx, conn, s, expectedRun, expectedPlacement)
		}); err != nil {
			return lockedSubmission{}, err
		}
		attempt, err := newPreparedSubmission(request, bounded.instant(), c)
		if err != nil {
			return lockedSubmission{}, err
		}
		held, acquired, err := host.lock(s, expectedPlacement, bounded)
		if err != nil {
			return lockedSubmission{}, err
		}
		if !acquired {
			return lockedSubmission{}, runtimeError("pane input remained busy before submission")
		}
		if _, err := bounded.remainingAt(c.now()); err != nil {
			held.close()
			return lockedSubmission{}, err
		}
		return lockedSubmission{attempt: attempt, held: held, deadline: bounded}, nil
	}()
	if err != nil {
		return none[submissionResult](), lockedSubmission{}, scope.failureOf(err)
	}
	return none[submissionResult](), locked, nil
}

func submitWrappedInputUnderPaneLockWithHost(ctx context.Context, conn *dbConn, s settings, expectedRun runRecord, expectedPlacement placement, held *paneLock, attempt preparedSubmission, d deadline, host inputCommandHost) (submissionResult, *inputFailure) {
	c := host.clock()
	bounded, err := submissionDeadline(ctx, deadlineUntil(earlier(d.instant(), attempt.deadline)), c)
	if err != nil {
		return submissionResult{}, submissionContext(expectedRun.runID, attempt.request).failureOf(err)
	}
	identity := permissionReceiptIdentityForRun(expectedRun)
	retained, failure := findSubmission(ctx, conn, identity, attempt.request, bounded, c)
	if failure != nil {
		return submissionResult{}, failure
	}
	if history, historical := historicalSubmission(retained).get(); historical {
		return history, nil
	}
	scope := submissionContext(expectedRun.runID, attempt.request)
	scope.inputID = inputIDOf(retained)
	validateTarget := func(ctx context.Context, conn *dbConn) (struct{}, error) {
		if err := held.validateBinding(s.databaseURL, expectedPlacement); err != nil {
			return struct{}{}, err
		}
		return struct{}{}, validateSubmissionTarget(ctx, conn, s, expectedRun, expectedPlacement)
	}
	result, err := func() (submissionResult, error) {
		if _, err := bounded.remainingAt(c.now()); err != nil {
			return submissionResult{}, err
		}
		if err := held.validateBinding(s.databaseURL, expectedPlacement); err != nil {
			return submissionResult{}, err
		}
		if _, err := withDatabaseBudget(ctx, conn, bounded, func(ctx context.Context, conn *dbConn) (struct{}, error) {
			return struct{}{}, validateSubmissionTarget(ctx, conn, s, expectedRun, expectedPlacement)
		}); err != nil {
			return submissionResult{}, err
		}
		if _, err := host.health(expectedRun, bounded.instant()); err != nil {
			return submissionResult{}, err
		}
		if _, err := bounded.remainingAt(c.now()); err != nil {
			return submissionResult{}, err
		}
		if _, err := withDatabaseBudget(ctx, conn, bounded, func(ctx context.Context, conn *dbConn) (struct{}, error) {
			return inTransaction(conn, transactionImmediate, func() (struct{}, error) {
				if _, err := validateTarget(ctx, conn); err != nil {
					return struct{}{}, err
				}
				changed, err := conn.execute(
					"UPDATE member_runs SET last_health_at=?1 WHERE run_id=?2 AND owner_epoch=?3",
					wallStamp(c), expectedRun.runID, expectedRun.ownerEpoch,
				)
				if err != nil {
					return struct{}{}, err
				}
				if err := ensure(changed == 1, "submission health ownership changed"); err != nil {
					return struct{}{}, err
				}
				_, err = bounded.remainingAt(c.now())
				return struct{}{}, err
			})
		}); err != nil {
			return submissionResult{}, err
		}
		var operation inputControlOperation
		switch attempt.request.InputKind {
		case inputKindPrompt:
			operation = submitPromptOperation{input: attempt.request}
		case inputKindExec:
			operation = submitExecOperation{input: attempt.request}
		default:
			panic("invalid inputKind")
		}
		request := controlRequest(expectedRun, attempt.expiration, operation)
		if err := held.validateBinding(s.databaseURL, expectedPlacement); err != nil {
			return submissionResult{}, err
		}
		if _, err := bounded.remainingAt(c.now()); err != nil {
			return submissionResult{}, err
		}
		if err := held.release(); err != nil {
			return submissionResult{}, err
		}
		answer, err := host.exchange(expectedRun, request, bounded.instant())
		if err != nil {
			return submissionResult{}, err
		}
		reply, isSubmission := answer.(submissionResult)
		if !isSubmission {
			return submissionResult{}, runtimeError("ordinary submission requires a submission reply")
		}
		recorded, failure := findSubmission(ctx, conn, identity, attempt.request, bounded, c)
		if failure != nil {
			return submissionResult{}, failure.asError()
		}
		scope.inputID = inputIDOf(recorded)
		runnerOwned := reply.Status == submissionStatusAccepted || reply.Status == submissionStatusQueued
		bookkeeping := deadlineUntil(later(bounded.instant(), c.now().Add(time.Second)))
		reacquired, err := held.reacquireWithin(bookkeeping)
		if err != nil {
			return submissionResult{}, err
		}
		if !reacquired && !runnerOwned {
			if inputID, set := scope.inputID.get(); set {
				return submissionResult{}, runtimeError("pane input lock remained busy after the runner reported " + reply.Status.asStr() +
					"; inspect the result with cafleet input show " + strconv.FormatInt(inputID, 10) + " --json")
			}
			return submissionResult{}, runtimeError("pane input lock remained busy after the runner reported " + reply.Status.asStr())
		}
		if runnerOwned {
			_, err = completeDatabaseBudget(ctx, conn, bounded, validateTarget)
		} else {
			_, err = withDatabaseBudget(ctx, conn, bounded, validateTarget)
		}
		if err != nil {
			return submissionResult{}, err
		}
		if err := validateReplyAgainstDurable(reply, recorded, replyMessages{
			"submission reply input identity mismatch",
			"submission reply contradicts durable state",
			"submission outcome requires a durable input identity",
			"submission reply has no matching durable input",
		}); err != nil {
			return submissionResult{}, err
		}
		if !runnerOwned {
			if _, err := bounded.remainingAt(c.now()); err != nil {
				return submissionResult{}, err
			}
		}
		return reply, nil
	}()
	if err != nil {
		return submissionResult{}, scope.failureOf(err)
	}
	return result, nil
}

func earlier(a, b time.Time) time.Time {
	if b.Before(a) {
		return b
	}
	return a
}

func later(a, b time.Time) time.Time {
	if b.After(a) {
		return b
	}
	return a
}

func validateReceiptTarget(ctx context.Context, conn *dbConn, s settings, expected runRecord, p placement) error {
	if err := validateReceiptDatabase(conn, s); err != nil {
		return err
	}
	current, err := revalidateRunSnapshot(ctx, conn, expected, p, snapshotPolicyHistoricalReceipt, snapshotMessages{
		runChanged:       "receipt run identity changed",
		placementChanged: "receipt member placement changed",
	})
	if err != nil {
		return err
	}
	_, err = validateRunPaths(ctx, conn, current)
	return err
}

func validateReceiptDatabase(conn *dbConn, s settings) error {
	return validateConfiguredDatabase(conn, s, "file-backed receipt database required", "receipt connection and configured database differ")
}

func validateConfiguredDatabase(conn *dbConn, s settings, memoryMessage, mismatchMessage string) error {
	selectedPath := conn.path()
	if selectedPath == "" {
		return runtimeError(memoryMessage)
	}
	selected, err := canonicalizePath(selectedPath)
	if err != nil {
		return ioError(ioErrorText(err))
	}
	configuredPath, err := databasePath(s.databaseURL)
	if err != nil {
		return err
	}
	configured, err := canonicalizePath(configuredPath)
	if err != nil {
		return ioError(ioErrorText(err))
	}
	return ensure(selected == configured, mismatchMessage)
}

func reconcileInput(ctx context.Context, conn *dbConn, s settings, inputID int64, disposition inputDisposition, evidenceRef string, d deadline) (inputOperation, *inputFailure) {
	return reconcileReceiptWithHost(ctx, conn, s, inputID, disposition, evidenceRef, d, nativeInputHost{})
}

func reconcileReceiptWithHost(ctx context.Context, conn *dbConn, s settings, inputID int64, disposition inputDisposition, evidenceRef string, d deadline, host inputCommandHost) (inputOperation, *inputFailure) {
	c := host.clock()
	scope := inputContext{}
	if inputID > 0 {
		scope.inputID = some(inputID)
	}
	recorded, err := func() (inputOperation, error) {
		bounded, err := submissionDeadline(ctx, d, c)
		if err != nil {
			return inputOperation{}, err
		}
		if err := usage(inputID > 0, "positive reconciliation input ID required"); err != nil {
			return inputOperation{}, err
		}
		if err := usage(strings.TrimSpace(evidenceRef) != "", "nonempty reconciliation evidence required"); err != nil {
			return inputOperation{}, err
		}
		type target struct {
			input     inputOperation
			run       runRecord
			placement opt[placement]
		}
		loaded, err := withDatabaseBudget(ctx, conn, bounded, func(ctx context.Context, conn *dbConn) (target, error) {
			return inTransaction(conn, transactionDeferred, func() (target, error) {
				if err := validateReceiptDatabase(conn, s); err != nil {
					return target{}, err
				}
				input, err := getInput(ctx, conn, inputID)
				if err != nil {
					return target{}, err
				}
				scope.runID = some(input.runID)
				scope.operationKey = some(input.operationKey)
				recordedRun, err := getRun(ctx, conn, input.runID)
				if err != nil {
					return target{}, err
				}
				run, err := validateOwner(ctx, conn, recordedRun.owner())
				if err != nil {
					return target{}, err
				}
				current := none[placement]()
				if input.state == inputStateSettled {
					if err := ensure(input.disposition == some(disposition), "settled input disposition changed"); err != nil {
						return target{}, err
					}
				} else if current, err = receiptPlacement(ctx, conn, run); err != nil {
					return target{}, err
				}
				if _, err := bounded.remainingAt(c.now()); err != nil {
					return target{}, err
				}
				return target{input: input, run: run, placement: current}, nil
			})
		})
		if err != nil {
			return inputOperation{}, err
		}
		original, run := loaded.input, loaded.run
		if original.state == inputStateSettled {
			return original, nil
		}
		livePlacement, live := loaded.placement.get()
		if !live {
			return inputOperation{}, runtimeError("receipt reconciliation requires the member's live runner; the input stays uncertain")
		}
		expiration, err := inputExpirationCapture(bounded.instant(), c)
		if err != nil {
			return inputOperation{}, err
		}
		held, acquired, err := host.lock(s, livePlacement, bounded)
		if err != nil {
			return inputOperation{}, err
		}
		if !acquired {
			return inputOperation{}, runtimeError("receipt pane input remained busy")
		}
		defer held.close()
		readTarget := func() (inputOperation, error) {
			return withDatabaseBudget(ctx, conn, bounded, func(ctx context.Context, conn *dbConn) (inputOperation, error) {
				return inTransaction(conn, transactionDeferred, func() (inputOperation, error) {
					if err := held.validateBinding(s.databaseURL, livePlacement); err != nil {
						return inputOperation{}, err
					}
					if err := validateReceiptTarget(ctx, conn, s, run, livePlacement); err != nil {
						return inputOperation{}, err
					}
					input, err := getInput(ctx, conn, inputID)
					if err != nil {
						return inputOperation{}, err
					}
					if err := ensure(input.sameIdentity(original), "receipt immutable input identity changed"); err != nil {
						return inputOperation{}, err
					}
					_, err = bounded.remainingAt(c.now())
					return input, err
				})
			})
		}
		if _, err := readTarget(); err != nil {
			return inputOperation{}, err
		}
		if _, err := host.health(run, bounded.instant()); err != nil {
			return inputOperation{}, err
		}
		if _, err := bounded.remainingAt(c.now()); err != nil {
			return inputOperation{}, err
		}
		if _, err := readTarget(); err != nil {
			return inputOperation{}, err
		}
		request := func(action receiptAction) inputControlRequest {
			return controlRequest(run, expiration, receiptOperation{action: action})
		}
		reply, err := host.exchange(run, request(receiptQuery{OperationKey: original.operationKey}), bounded.instant())
		if err != nil {
			return inputOperation{}, err
		}
		present, isPresent := reply.(receiptPresent)
		if err := ensure(isPresent && present.InputID == inputID, "receipt query has no matching durable input identity"); err != nil {
			return inputOperation{}, err
		}
		if _, err := bounded.remainingAt(c.now()); err != nil {
			return inputOperation{}, err
		}
		if _, err := readTarget(); err != nil {
			return inputOperation{}, err
		}
		reconcile := receiptReconcile{InputID: inputID, Expected: original.state, Disposition: disposition, EvidenceRef: evidenceRef}
		if _, err := host.exchange(run, request(reconcile), bounded.instant()); err != nil {
			return inputOperation{}, err
		}
		settled, err := readTarget()
		if err != nil {
			return inputOperation{}, err
		}
		if err := ensure(
			settled.state == inputStateSettled && settled.disposition == some(disposition) && settled.evidenceRef == some(evidenceRef),
			"receipt settlement acknowledgment has no matching durable settlement",
		); err != nil {
			return inputOperation{}, err
		}
		if _, err := bounded.remainingAt(c.now()); err != nil {
			return inputOperation{}, err
		}
		if _, err := host.exchange(run, request(receiptRetire{InputID: inputID}), bounded.instant()); err != nil {
			return inputOperation{}, err
		}
		if _, err := bounded.remainingAt(c.now()); err != nil {
			return inputOperation{}, err
		}
		recorded, err := readTarget()
		if err != nil {
			return inputOperation{}, err
		}
		if err := ensure(recorded == settled, "receipt changed after retirement"); err != nil {
			return inputOperation{}, err
		}
		return recorded, nil
	}()
	if err != nil {
		return inputOperation{}, scope.failureOf(err)
	}
	return recorded, nil
}

func usage(ok bool, description string) error {
	if ok {
		return nil
	}
	return usageError(description)
}

func validateEvidenceReference(value string) error {
	return usage(
		strings.TrimSpace(value) != "" && len(value) <= 2048 &&
			!strings.ContainsFunc(value, func(r rune) bool { return r <= '\u001f' || r == '\u007f' }),
		"a nonblank evidence reference of 1-2048 UTF-8 bytes without control characters is required",
	)
}

func validateRequestToken(value string) error {
	valid := len(value) == 64
	for i := 0; valid && i < len(value); i++ {
		b := value[i]
		valid = (b >= '0' && b <= '9') || (b >= 'a' && b <= 'f')
	}
	return usage(valid, "a lowercase 64-hex request token is required")
}

func validateOperationKey(value string) error {
	return usage(strings.TrimSpace(value) != "", "a nonblank operation key is required")
}

func validateRespond(input permissionRespondInput) error {
	if err := usage(
		!slices.ContainsFunc([]int64{
			input.InteractionID,
			input.EventID,
			input.DirectorMemberID,
			input.ExpectedRunID,
			input.ExpectedOwnerEpoch,
		}, func(id int64) bool { return id <= 0 }),
		"positive permission request IDs and owner epoch are required",
	); err != nil {
		return err
	}
	return validateRequestToken(input.RequestToken)
}

func selectedRun(ctx context.Context, conn *dbConn, runID int64) (runRecord, error) {
	return selectedRunReporting(ctx, conn, runID, "member_runs.database_identity_json")
}

func selectedRunReporting(ctx context.Context, conn *dbConn, runID int64, field string) (runRecord, error) {
	identity, err := databaseIdentity(ctx, conn)
	if err != nil {
		return runRecord{}, err
	}
	run, err := getRun(ctx, conn, runID)
	if err != nil {
		return runRecord{}, err
	}
	if !jsonEqual(identity, run.databaseIdentityJSON) {
		return runRecord{}, invalidStoredValueError(field, "selected database identity mismatch")
	}
	return run, nil
}

func association(ctx context.Context, conn *dbConn, run runRecord, interactionID, eventID, director int64) error {
	view, err := getEvent(ctx, conn, eventID)
	if err != nil {
		return err
	}
	interaction, interacted := view.interaction.get()
	return ensure(
		view.event.runID == run.runID &&
			view.event.interactionID == some(interactionID) &&
			view.event.kind == runtimeEventKindInteractionOpened &&
			view.event.notificationState == notificationStateInbox &&
			director == run.directorMemberID &&
			interacted &&
			(interaction.classification == interactionClassPermissionRequest || interaction.classification == interactionClassQuestion),
		"permission run/event/interaction/Director association mismatch",
	)
}

func responseContext(input permissionRespondInput) inputContext {
	return inputContext{runID: some(input.ExpectedRunID), interactionID: some(input.InteractionID)}
}

func prepareResponse(ctx context.Context, conn *dbConn, input permissionRespondInput, d deadline, c clock) (runRecord, runtimeEventView, error) {
	type prepared struct {
		run      runRecord
		original runtimeEventView
	}
	result, err := withDatabaseBudget(ctx, conn, d, func(ctx context.Context, conn *dbConn) (prepared, error) {
		return inTransaction(conn, transactionDeferred, func() (prepared, error) {
			run, err := selectedRun(ctx, conn, input.ExpectedRunID)
			if err != nil {
				return prepared{}, err
			}
			if err := association(ctx, conn, run, input.InteractionID, input.EventID, input.DirectorMemberID); err != nil {
				return prepared{}, err
			}
			if err := ensure(input.ExpectedOwnerEpoch == run.ownerEpoch, "permission owner epoch changed"); err != nil {
				return prepared{}, err
			}
			original, err := inspectionAssociation(ctx, conn, run, input.inspection())
			if err != nil {
				return prepared{}, err
			}
			_, err = d.remainingAt(c.now())
			return prepared{run: run, original: original}, err
		})
	})
	return result.run, result.original, err
}

func validateResponseIdentity(reply permissionResponseResult, run runRecord, input permissionRespondInput) error {
	return ensure(
		reply.MemberID == run.memberID &&
			reply.RunID == input.ExpectedRunID &&
			reply.OwnerEpoch == input.ExpectedOwnerEpoch &&
			reply.InteractionID == input.InteractionID &&
			reply.EventID == input.EventID &&
			reply.Key == input.Key &&
			!reply.Submission.InputID.set,
		"permission response reply identity mismatch",
	)
}

func respondPermissionWithHost(ctx context.Context, conn *dbConn, s settings, input permissionRespondInput, d deadline, host inputCommandHost) (permissionResponseResult, *inputFailure) {
	c := host.clock()
	scope := responseContext(input)
	result, err := func() (permissionResponseResult, error) {
		if err := validateRespond(input); err != nil {
			return permissionResponseResult{}, err
		}
		bounded, err := submissionDeadline(ctx, d, c)
		if err != nil {
			return permissionResponseResult{}, err
		}
		run, original, err := prepareResponse(ctx, conn, input, bounded, c)
		if err != nil {
			return permissionResponseResult{}, err
		}
		target, err := withDatabaseBudget(ctx, conn, bounded, func(ctx context.Context, conn *dbConn) (placement, error) {
			return permissionPlacement(ctx, conn, run)
		})
		if err != nil {
			return permissionResponseResult{}, err
		}
		expiration, err := inputExpirationCapture(bounded.instant(), c)
		if err != nil {
			return permissionResponseResult{}, err
		}
		held, acquired, err := host.lock(s, target, bounded)
		if err != nil {
			return permissionResponseResult{}, err
		}
		if !acquired {
			return permissionResponseResult{}, runtimeError("permission pane input lock is busy")
		}
		defer held.close()
		validate := func() error {
			if _, err := bounded.remainingAt(c.now()); err != nil {
				return err
			}
			if err := held.validateBinding(s.databaseURL, target); err != nil {
				return err
			}
			_, err := withDatabaseBudget(ctx, conn, bounded, func(ctx context.Context, conn *dbConn) (struct{}, error) {
				return inTransaction(conn, transactionDeferred, func() (struct{}, error) {
					if err := validateSubmissionTarget(ctx, conn, s, run, target); err != nil {
						return struct{}{}, err
					}
					view, err := inspectionAssociation(ctx, conn, run, input.inspection())
					if err != nil {
						return struct{}{}, err
					}
					if err := ensure(sameAssociation(view, original), "permission response association changed"); err != nil {
						return struct{}{}, err
					}
					_, err = bounded.remainingAt(c.now())
					return struct{}{}, err
				})
			})
			return err
		}
		if err := bracketHealth(host, run, bounded.instant(), validate); err != nil {
			return permissionResponseResult{}, err
		}
		request := controlRequest(run, expiration, permissionRespondOperation{input: input})
		answer, err := host.exchange(run, request, bounded.instant())
		if err != nil {
			return permissionResponseResult{}, err
		}
		reply, isResponse := answer.(permissionResponseResult)
		if !isResponse {
			return permissionResponseResult{}, runtimeError("permission response requires its typed reply")
		}
		if err := validateResponseIdentity(reply, run, input); err != nil {
			return permissionResponseResult{}, err
		}
		if _, err := withDatabaseBudget(ctx, conn, bounded, func(ctx context.Context, conn *dbConn) (struct{}, error) {
			return inTransaction(conn, transactionDeferred, func() (struct{}, error) {
				if err := held.validateBinding(s.databaseURL, target); err != nil {
					return struct{}{}, err
				}
				if err := validateSubmissionTarget(ctx, conn, s, run, target); err != nil {
					return struct{}{}, err
				}
				if err := association(ctx, conn, run, input.InteractionID, input.EventID, input.DirectorMemberID); err != nil {
					return struct{}{}, err
				}
				_, err := bounded.remainingAt(c.now())
				return struct{}{}, err
			})
		}); err != nil {
			return permissionResponseResult{}, err
		}
		if _, err := bounded.remainingAt(c.now()); err != nil {
			return permissionResponseResult{}, err
		}
		return reply, nil
	}()
	if err != nil {
		return permissionResponseResult{}, scope.failureOf(err)
	}
	return result, nil
}

func inspectPermission(ctx context.Context, conn *dbConn, s settings, input permissionInspectInput, d deadline) (permissionInspection, *inputFailure) {
	return inspectPermissionWithHost(ctx, conn, s, input, d, nativeInputHost{})
}

func inspectPermissionWithHost(ctx context.Context, conn *dbConn, s settings, input permissionInspectInput, d deadline, host inputCommandHost) (permissionInspection, *inputFailure) {
	c := host.clock()
	scope := inputContext{interactionID: some(input.InteractionID)}
	result, err := func() (permissionInspection, error) {
		if err := usage(
			input.InteractionID > 0 && input.EventID > 0 && input.DirectorMemberID > 0,
			"positive permission request IDs are required",
		); err != nil {
			return permissionInspection{}, err
		}
		bounded, err := submissionDeadline(ctx, d, c)
		if err != nil {
			return permissionInspection{}, err
		}
		type selected struct {
			run       runRecord
			placement placement
			original  runtimeEventView
		}
		target, err := withDatabaseBudget(ctx, conn, bounded, func(ctx context.Context, conn *dbConn) (selected, error) {
			return inTransaction(conn, transactionDeferred, func() (selected, error) {
				exists, err := queryRowOptional(conn, "SELECT 1 FROM member_interactions WHERE interaction_id=?1", []any{input.InteractionID}, func(row sqlRow) (int64, error) {
					return row.int64(0)
				})
				if err != nil {
					return selected{}, err
				}
				if !exists.set {
					return selected{}, appErrorf("Interaction %d not found", input.InteractionID)
				}
				view, err := getEvent(ctx, conn, input.EventID)
				if err != nil {
					return selected{}, err
				}
				run, err := selectedRunReporting(ctx, conn, view.event.runID, "member_runs.database_identity_json")
				if err != nil {
					return selected{}, err
				}
				scope.runID = some(run.runID)
				target, err := permissionPlacement(ctx, conn, run)
				if err != nil {
					return selected{}, err
				}
				if err := validateSubmissionTarget(ctx, conn, s, run, target); err != nil {
					return selected{}, err
				}
				original, err := inspectionAssociation(ctx, conn, run, input)
				if err != nil {
					return selected{}, err
				}
				_, err = bounded.remainingAt(c.now())
				return selected{run: run, placement: target, original: original}, err
			})
		})
		if err != nil {
			return permissionInspection{}, err
		}
		run, original := target.run, target.original
		expiration, err := inputExpirationCapture(bounded.instant(), c)
		if err != nil {
			return permissionInspection{}, err
		}
		held, acquired, err := host.lock(s, target.placement, bounded)
		if err != nil {
			return permissionInspection{}, err
		}
		if !acquired {
			return permissionInspection{}, runtimeError("permission pane input lock is busy")
		}
		defer held.close()
		validate := func() error {
			if _, err := bounded.remainingAt(c.now()); err != nil {
				return err
			}
			if err := held.validateBinding(s.databaseURL, target.placement); err != nil {
				return err
			}
			_, err := withDatabaseBudget(ctx, conn, bounded, func(ctx context.Context, conn *dbConn) (struct{}, error) {
				return inTransaction(conn, transactionDeferred, func() (struct{}, error) {
					if err := validateSubmissionTarget(ctx, conn, s, run, target.placement); err != nil {
						return struct{}{}, err
					}
					view, err := inspectionAssociation(ctx, conn, run, input)
					if err != nil {
						return struct{}{}, err
					}
					if err := ensure(sameAssociation(view, original), "permission inspection association changed"); err != nil {
						return struct{}{}, err
					}
					_, err = bounded.remainingAt(c.now())
					return struct{}{}, err
				})
			})
			return err
		}
		if err := bracketHealth(host, run, bounded.instant(), validate); err != nil {
			return permissionInspection{}, err
		}
		request := controlRequest(run, expiration, permissionInspectOperation{input: input})
		answer, err := host.exchange(run, request, bounded.instant())
		if err != nil {
			return permissionInspection{}, err
		}
		if err := validate(); err != nil {
			return permissionInspection{}, err
		}
		reply, isInspection := answer.(permissionInspection)
		if !isInspection {
			return permissionInspection{}, runtimeError("permission inspection requires its typed reply")
		}
		if err := validateInspectionReply(reply, run, original); err != nil {
			return permissionInspection{}, err
		}
		if _, err := bounded.remainingAt(c.now()); err != nil {
			return permissionInspection{}, err
		}
		return reply, nil
	}()
	if err != nil {
		return permissionInspection{}, scope.failureOf(err)
	}
	return result, nil
}

// validateInspectionReply checks the runner's inspection against the durable interaction it
// answers and against its own capability rules.
func validateInspectionReply(reply permissionInspection, run runRecord, original runtimeEventView) error {
	interaction, interacted := original.interaction.get()
	if !interacted {
		panic("a validated permission association has its interaction")
	}
	messageID, messaged := original.event.messageID.get()
	if !messaged {
		panic("a validated permission association has its inbox message")
	}
	if err := ensure(
		reply.MessageID == messageID &&
			reply.EpisodeSeq == interaction.episodeSeq &&
			reply.Backend == run.backend &&
			reply.NativeVersion == run.nativeVersion,
		"permission inspection reply identity mismatch",
	); err != nil {
		return err
	}
	backend, backendIsString := jsonString(jsonIndex(reply.Evidence, "backend"))
	nativeVersion := none[string]()
	if version, isString := jsonString(jsonIndex(reply.Evidence, "native_version")); isString {
		nativeVersion = some(version)
	}
	screenSeq, screenIsUnsigned := jsonUint64(jsonIndex(reply.Evidence, "screen_seq"))
	if err := ensure(
		backendIsString && backend == run.backend &&
			nativeVersion == run.nativeVersion &&
			screenIsUnsigned && screenSeq == reply.OutputSeq,
		"permission inspection evidence identity mismatch",
	); err != nil {
		return err
	}
	missing, err := interactionMissingEvidence(interaction.classification, reply.Evidence)
	if err != nil {
		return err
	}
	if err := ensure(slices.Equal(reply.MissingEvidence, missing), "permission inspection evidence completeness mismatch"); err != nil {
		return err
	}
	switch reply.AnswerCapability {
	case coverageTested:
		var offered bool
		if interaction.classification == interactionClassQuestion {
			offered = len(reply.SupportedDecisions) == 0 && len(reply.SupportedChoices) > 0
		} else {
			offered = len(reply.SupportedDecisions) > 0 && len(reply.SupportedChoices) == 0
		}
		if err := ensure(
			reply.RequestToken.set && offered &&
				reply.Classification == some(interaction.classification) &&
				reply.Visibility == visibilityVisible &&
				!reply.CapabilityReason.set,
			"permission inspection capability fields disagree",
		); err != nil {
			return err
		}
	case coverageUnknown:
		reason, reasoned := reply.CapabilityReason.get()
		if err := ensure(
			!reply.RequestToken.set &&
				len(reply.SupportedDecisions) == 0 &&
				len(reply.SupportedChoices) == 0 &&
				reasoned && strings.TrimSpace(reason) != "",
			"permission inspection unavailable fields disagree",
		); err != nil {
			return err
		}
	default:
		panic("invalid coverage")
	}
	if rule, set := reply.AnswerRuleID.get(); set {
		return ensure(strings.TrimSpace(rule) != "" && len(rule) <= 256, "permission inspection answer rule is invalid")
	}
	return nil
}

type snapshotPolicy uint8

const (
	snapshotPolicyLive snapshotPolicy = iota + 1
	snapshotPolicyHistoricalReceipt
)

type snapshotMessages struct {
	runChanged       string
	placementChanged string
}

func revalidateRunSnapshot(ctx context.Context, conn *dbConn, expected runRecord, p placement, policy snapshotPolicy, messages snapshotMessages) (runRecord, error) {
	current, err := validateOwner(ctx, conn, expected.owner())
	if err != nil {
		return runRecord{}, err
	}
	sameRun := jsonEqual(stableRunIdentity(current), stableRunIdentity(expected))
	switch policy {
	case snapshotPolicyLive:
		if err := ensure(
			sameRun && current.status == runStatusRunning && current.ownerKind == ownerKindRunner &&
				current.nativeVersion == expected.nativeVersion,
			messages.runChanged,
		); err != nil {
			return runRecord{}, err
		}
		fleet, err := activeMemberFleet(ctx, conn, expected.memberID)
		if err != nil {
			return runRecord{}, err
		}
		if err := ensure(fleet == some(expected.fleetID), "input member is inactive"); err != nil {
			return runRecord{}, err
		}
		member, err := getMember(ctx, conn, expected.memberID, expected.fleetID)
		if err != nil {
			return runRecord{}, err
		}
		record, found := member.get()
		if !found {
			return runRecord{}, runtimeError("input member is absent")
		}
		if err := ensure(record.kind == memberKindMember && record.placement == p, messages.placementChanged); err != nil {
			return runRecord{}, err
		}
	case snapshotPolicyHistoricalReceipt:
		if err := ensure(sameRun, messages.runChanged); err != nil {
			return runRecord{}, err
		}
		recorded, err := receiptPlacement(ctx, conn, current)
		if err != nil {
			return runRecord{}, err
		}
		if err := ensure(recorded == some(p), messages.placementChanged); err != nil {
			return runRecord{}, err
		}
	default:
		panic("invalid snapshotPolicy")
	}
	return current, nil
}

func receiptPlacement(ctx context.Context, conn *dbConn, run runRecord) (opt[placement], error) {
	type membership struct {
		status  memberStatus
		deleted bool
	}
	recorded, err := queryRow(conn,
		"SELECT m.status,f.deleted_at FROM members m JOIN fleets f ON f.fleet_id=m.fleet_id WHERE m.member_id=?1 AND m.fleet_id=?2",
		[]any{run.memberID, run.fleetID},
		func(row sqlRow) (membership, error) {
			text, err := row.text(0)
			if err != nil {
				return membership{}, err
			}
			status, err := memberStatusTryFrom(text)
			if err != nil {
				return membership{}, err
			}
			deleted, err := row.optText(1)
			return membership{status: status, deleted: deleted.set}, err
		})
	if err != nil {
		return none[placement](), err
	}
	current, err := placementOf(ctx, conn, run.memberID)
	if err != nil {
		return none[placement](), err
	}
	if err := ensure(
		current.set || recorded.status != memberStatusActive || recorded.deleted,
		"active receipt member requires its recorded placement",
	); err != nil {
		return none[placement](), err
	}
	return current, nil
}
