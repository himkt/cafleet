package core

import (
	"context"
	"fmt"
	"slices"
)

func mapInput(row sqlRow) (inputOperation, error) {
	var input inputOperation
	var err error
	column := row.columnIndex
	if input.inputID, err = row.int64(column("input_id")); err != nil {
		return inputOperation{}, err
	}
	if input.runID, err = row.int64(column("run_id")); err != nil {
		return inputOperation{}, err
	}
	if input.operationKey, err = row.text(column("operation_key")); err != nil {
		return inputOperation{}, err
	}
	if input.kind, err = rowStoredEnum(row, column("kind"), inputKindTryFrom); err != nil {
		return inputOperation{}, err
	}
	if input.payloadDigest, err = row.text(column("payload_digest")); err != nil {
		return inputOperation{}, err
	}
	if input.state, err = rowStoredEnum(row, column("state"), inputStateTryFrom); err != nil {
		return inputOperation{}, err
	}
	if input.createdAt, err = row.text(column("created_at")); err != nil {
		return inputOperation{}, err
	}
	if input.updatedAt, err = row.text(column("updated_at")); err != nil {
		return inputOperation{}, err
	}
	if input.disposition, err = rowOptStoredEnum(row, column("disposition"), inputDispositionTryFrom); err != nil {
		return inputOperation{}, err
	}
	if input.evidenceRef, err = row.optText(column("evidence_ref")); err != nil {
		return inputOperation{}, err
	}
	if input.deliveryMethod, err = rowOptStoredEnum(row, column("delivery_method"), deliveryMethodTryFrom); err != nil {
		return inputOperation{}, err
	}
	if input.deliveryReason, err = row.optText(column("delivery_reason")); err != nil {
		return inputOperation{}, err
	}
	if input.deferredAt, err = row.optText(column("deferred_at")); err != nil {
		return inputOperation{}, err
	}
	if input.deferReason, err = row.optText(column("defer_reason")); err != nil {
		return inputOperation{}, err
	}
	if input.stopCause, err = rowOptStoredEnum(row, column("stop_cause"), stopCauseTryFrom); err != nil {
		return inputOperation{}, err
	}
	return input, nil
}

// rowOptStoredEnum reads a nullable stored enum column.
func rowOptStoredEnum[T comparable](row sqlRow, column int, tryFrom func(string) (T, error)) (opt[T], error) {
	text, err := row.optText(column)
	if err != nil {
		return none[T](), err
	}
	value, set := text.get()
	if !set {
		return none[T](), nil
	}
	parsed, err := tryFrom(value)
	if err != nil {
		return none[T](), err
	}
	return some(parsed), nil
}

func (i inputOperation) delivery() deliveryProjection {
	projection, err := projectDelivery(i)
	if err != nil {
		panic("validated input receipts project their delivery: " + err.Error())
	}
	return projection
}

var fallbackReasons = []string{"submission_plan_unavailable"}

// projectDelivery is the Rust delivery_projection.
func projectDelivery(input inputOperation) (deliveryProjection, error) {
	unknown := brokerRuntimeInvalid("member_input_operations.delivery_method", "unknown stored delivery combination")
	method := input.deliveryMethod
	_, hasReason := input.deliveryReason.get()
	recorded := func() (opt[string], error) {
		selected, set := method.get()
		switch {
		case set && selected == deliveryMethodProfile && !hasReason:
			return none[string](), nil
		case set && selected == deliveryMethodFallback && hasReason:
			reason, _ := input.deliveryReason.get()
			if slices.Contains(fallbackReasons, reason) {
				return input.deliveryReason, nil
			}
			return none[string](), unknown
		default:
			return none[string](), unknown
		}
	}
	_, methodSet := method.get()
	_, deferred := input.deferredAt.get()
	disposition, disposed := input.disposition.get()
	var status deliveryStatus
	var reason opt[string]
	var err error
	switch {
	case input.state == inputStateReady && deferred:
		if err := brokerRuntimeRequire(!methodSet && !hasReason, "member_input_operations.delivery_method", "a queued input has no selected method"); err != nil {
			return deliveryProjection{}, err
		}
		status, reason = deliveryStatusQueued, input.deferReason
	case input.state == inputStateReady:
		if err := brokerRuntimeRequire(!methodSet && !hasReason, "member_input_operations.delivery_method", "a ready input has no selected method"); err != nil {
			return deliveryProjection{}, err
		}
		status, reason = deliveryStatusHeld, some("zero_bytes_ready")
	case input.state == inputStateInProgress || input.state == inputStateUncertain ||
		(input.state == inputStateSettled && disposed && disposition == inputDispositionAbandonedUncertain):
		status = deliveryStatusUncertain
		if reason, err = recorded(); err != nil {
			return deliveryProjection{}, err
		}
	case input.state == inputStateAccepted ||
		(input.state == inputStateSettled && disposed && disposition == inputDispositionSubmitted):
		status = deliveryStatusSubmitted
		if reason, err = recorded(); err != nil {
			return deliveryProjection{}, err
		}
	case input.state == inputStateSettled && disposed && disposition == inputDispositionNotSubmitted:
		status = deliveryStatusNotSubmitted
		if methodSet {
			if reason, err = recorded(); err != nil {
				return deliveryProjection{}, err
			}
		} else if stored, set := input.deliveryReason.get(); set {
			reason = some(stored)
		} else {
			reason = some("method_unrecorded")
		}
	case input.state == inputStateSettled:
		return deliveryProjection{}, unknown
	default:
		panic("invalid inputState")
	}
	return deliveryProjection{Status: status, Method: method, Reason: reason}, nil
}

func validateInput(ctx context.Context, conn *dbConn, input inputOperation) (inputOperation, error) {
	if _, err := getRun(ctx, conn, input.runID); err != nil {
		return inputOperation{}, err
	}
	_, hasStop := input.stopCause.get()
	stopAllowed := !hasStop || input.state == inputStateUncertain || input.state == inputStateSettled
	if err := brokerRuntimeRequire(stopAllowed, "member_input_operations.stop_cause", "a stop cause requires an uncertain or settled input"); err != nil {
		return inputOperation{}, err
	}
	if _, err := projectDelivery(input); err != nil {
		return inputOperation{}, err
	}
	return input, nil
}

func inputPayloadDigest(runID int64, input submitInput) string {
	payload, err := encodeJSON(jsonObject{{"run_id", runID}, {"kind", input.InputKind.asStr()}, {"literal", input.Literal}})
	if err != nil {
		panic("an input payload encodes: " + err.Error())
	}
	canonical, err := canonicalJSON(payload)
	if err != nil {
		panic("an encoded input payload is valid JSON: " + err.Error())
	}
	return sha256Hex([]byte(canonical))
}

type reservationSelection struct {
	method deliveryMethod
	reason opt[string]
}

// newReservation is how a new ordinary receipt is reserved: the delivery selected before its
// first byte, or a zero-byte queue admission with its defer reason.
type newReservation struct {
	selection   opt[reservationSelection]
	deferReason opt[string]
}

func beginDeferredAttempt(ctx context.Context, conn *dbConn, owner runOwner, inputID int64, method deliveryMethod, reason opt[string]) (inputOperation, error) {
	return ownedTransaction(ctx, conn, owner, func(ctx context.Context, conn *dbConn, run runRecord) (inputOperation, error) {
		input, err := ownedInput(ctx, conn, run, inputID)
		if err != nil {
			return inputOperation{}, err
		}
		_, deferred := input.deferredAt.get()
		if err := brokerRuntimeRequire(input.state == inputStateReady && deferred, "member_input_operations.state", "a deferred attempt starts from its queued receipt"); err != nil {
			return inputOperation{}, err
		}
		return updateInput(ctx, conn, inputID,
			"UPDATE member_input_operations SET state='in_progress',updated_at=?1,delivery_method=?2,delivery_reason=?3 WHERE input_id=?4 AND state='ready'",
			[]any{nowStamp(), method.asStr(), reason, inputID}, casFailed)
	})
}

// hasUnresolvedOrdinaryAttempt reports whether an ordinary attempt of the run, deferred or
// immediate, may have written bytes and awaits reconciliation, which stops automatic ordinary
// dispatch until it settles.
func hasUnresolvedOrdinaryAttempt(_ context.Context, conn *dbConn, runID int64) (bool, error) {
	return queryRow(conn,
		"SELECT EXISTS(SELECT 1 FROM member_input_operations WHERE run_id=?1 AND kind IN ('prompt','exec') AND state IN ('in_progress','uncertain'))",
		[]any{runID}, func(row sqlRow) (bool, error) { return row.bool(0) })
}

// requeueZeroByteImmediate returns a reserved immediate request that wrote zero bytes to the
// queue, clearing its unexecuted selection and recording why it was deferred.
func requeueZeroByteImmediate(ctx context.Context, conn *dbConn, owner runOwner, inputID int64, deferReason string) (inputOperation, error) {
	return ownedTransaction(ctx, conn, owner, func(ctx context.Context, conn *dbConn, run runRecord) (inputOperation, error) {
		input, err := ownedInput(ctx, conn, run, inputID)
		if err != nil {
			return inputOperation{}, err
		}
		_, deferred := input.deferredAt.get()
		if err := brokerRuntimeRequire(input.state == inputStateInProgress && !deferred, "member_input_operations.state", "only an immediate ordinary attempt returns to the queue"); err != nil {
			return inputOperation{}, err
		}
		now := nowStamp()
		return updateInput(ctx, conn, inputID,
			"UPDATE member_input_operations SET state='ready',updated_at=?1,delivery_method=NULL,delivery_reason=NULL,deferred_at=?1,defer_reason=?2 WHERE input_id=?3 AND state='in_progress'",
			[]any{now, deferReason, inputID}, casFailed)
	})
}

type unpublishedDeliveryResult struct {
	inputID   int64
	stage     string
	stopCause opt[stopCause]
}

func unpublishedDeliveryResults(ctx context.Context, conn *dbConn, runID int64) ([]unpublishedDeliveryResult, error) {
	ids, err := queryRows(conn,
		"SELECT input_id FROM member_input_operations WHERE run_id=?1 AND kind IN ('prompt','exec') AND (deferred_at IS NOT NULL OR stop_cause IS NOT NULL) AND input_id NOT IN (SELECT json_extract(payload_json,'$.input_id') FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_result') ORDER BY input_id",
		[]any{runID}, func(row sqlRow) (int64, error) { return row.int64(0) })
	if err != nil {
		return nil, err
	}
	results := []unpublishedDeliveryResult{}
	for _, inputID := range ids {
		input, err := getInput(ctx, conn, inputID)
		if err != nil {
			return nil, err
		}
		_, deferred := input.deferredAt.get()
		_, stopped := input.stopCause.get()
		if !deferred && stopped {
			results = append(results, unpublishedDeliveryResult{inputID: inputID, stage: "unknown", stopCause: input.stopCause})
			continue
		}
		delivery := input.delivery()
		var stage string
		switch delivery.Status {
		case deliveryStatusSubmitted:
			stage = "submitted"
		case deliveryStatusNotSubmitted:
			stage = "zero_bytes"
		case deliveryStatusUncertain:
			stage = "unknown"
		case deliveryStatusQueued, deliveryStatusHeld, deliveryStatusNotApplicable:
			continue
		default:
			panic("invalid deliveryStatus")
		}
		cause := none[stopCause]()
		if delivery.Status == deliveryStatusUncertain {
			cause = input.stopCause
		}
		results = append(results, unpublishedDeliveryResult{inputID: inputID, stage: stage, stopCause: cause})
	}
	return results, nil
}

// settleLostDeferred settles every queued receipt of a run whose runner was lost, after ownership
// fencing has established that its in-memory payload and writer are gone.
func settleLostDeferred(ctx context.Context, conn *dbConn, owner runOwner) error {
	_, err := ownedTransaction(ctx, conn, owner, func(ctx context.Context, conn *dbConn, run runRecord) (struct{}, error) {
		return struct{}{}, settleLostDeferredInTransaction(ctx, conn, run)
	})
	return err
}

// settleLostDeferredInTransaction is settleLostDeferred inside a caller's transaction that
// already holds the run's reconciler ownership.
func settleLostDeferredInTransaction(_ context.Context, conn *dbConn, run runRecord) error {
	if err := brokerRuntimeRequire(run.ownerKind == ownerKindReconciler, "owner_kind", "lost queued input settles under reconciler ownership"); err != nil {
		return err
	}
	_, err := conn.execute(
		"UPDATE member_input_operations SET state='settled',disposition='not_submitted',updated_at=?1,delivery_method=NULL,delivery_reason='runner_lost' WHERE run_id=?2 AND state='ready' AND deferred_at IS NOT NULL",
		nowStamp(), run.runID)
	return err
}

// settleZeroByteDeferred settles a queued input that wrote zero bytes as not submitted with its
// outcome reason.
func settleZeroByteDeferred(ctx context.Context, conn *dbConn, owner runOwner, inputID int64, expected inputState, reason string) (inputOperation, error) {
	if err := brokerRuntimeRequire(expected == inputStateReady || expected == inputStateInProgress, "member_input_operations.state", "only a zero-byte deferred input is cancelled"); err != nil {
		return inputOperation{}, err
	}
	return ownedTransaction(ctx, conn, owner, func(ctx context.Context, conn *dbConn, run runRecord) (inputOperation, error) {
		input, err := ownedInput(ctx, conn, run, inputID)
		if err != nil {
			return inputOperation{}, err
		}
		_, deferred := input.deferredAt.get()
		if err := brokerRuntimeRequire(input.state == expected && deferred, "member_input_operations.state", "deferred input state changed"); err != nil {
			return inputOperation{}, err
		}
		return updateInput(ctx, conn, inputID,
			"UPDATE member_input_operations SET state='settled',disposition='not_submitted',updated_at=?1,delivery_method=NULL,delivery_reason=?2 WHERE input_id=?3 AND state=?4",
			[]any{nowStamp(), reason, inputID, expected.asStr()}, casFailed)
	})
}

func getInput(ctx context.Context, conn *dbConn, inputID int64) (inputOperation, error) {
	rows, err := queryRows(conn, "SELECT * FROM member_input_operations WHERE input_id=?1", []any{inputID}, mapInput)
	if err != nil {
		return inputOperation{}, err
	}
	if len(rows) == 0 {
		return inputOperation{}, appError(fmt.Sprintf("Input %d not found", inputID))
	}
	return validateInput(ctx, conn, rows[0])
}

func findInput(ctx context.Context, conn *dbConn, runID int64, key string) (opt[inputOperation], error) {
	found, err := queryRowOptional(conn, "SELECT * FROM member_input_operations WHERE run_id=?1 AND operation_key=?2",
		[]any{runID, key}, mapInput)
	if err != nil {
		return none[inputOperation](), err
	}
	input, set := found.get()
	if !set {
		return none[inputOperation](), nil
	}
	validated, err := validateInput(ctx, conn, input)
	if err != nil {
		return none[inputOperation](), err
	}
	return some(validated), nil
}

func firstUncertainOrdinaryInput(_ context.Context, conn *dbConn, runID int64) (opt[int64], error) {
	return queryRow(conn,
		`SELECT MIN(input_id) FROM member_input_operations
         WHERE run_id=?1 AND state='uncertain' AND kind IN ('prompt','exec')`,
		[]any{runID}, func(row sqlRow) (opt[int64], error) { return row.optInt64(0) })
}

// inputReservation is a reserved receipt: newly inserted, or the existing one for the same key.
type inputReservation struct {
	operation inputOperation
	inserted  bool
}

func reserveInputWithProvenance(ctx context.Context, conn *dbConn, owner runOwner, request newInput, reservation newReservation) (inputReservation, error) {
	if err := brokerRuntimeNonblank(request.operationKey, "operation_key"); err != nil {
		return inputReservation{}, err
	}
	if err := brokerRuntimeHex(request.payloadDigest, 64, "payload_digest"); err != nil {
		return inputReservation{}, err
	}
	return ownedTransaction(ctx, conn, owner, func(ctx context.Context, conn *dbConn, run runRecord) (inputReservation, error) {
		existing, err := findInput(ctx, conn, run.runID, request.operationKey)
		if err != nil {
			return inputReservation{}, err
		}
		if input, found := existing.get(); found {
			same := input.payloadDigest == request.payloadDigest && input.kind == request.kind
			if err := brokerRuntimeRequire(same, "operation_key", "conflicting operation key reuse"); err != nil {
				return inputReservation{}, err
			}
			return inputReservation{operation: input, inserted: false}, nil
		}
		ownership, err := historicalOwnership(ctx, conn, run)
		if err != nil {
			return inputReservation{}, err
		}
		_, archived := ownership.get()
		if err := brokerRuntimeRequire(run.status == runStatusRunning && !archived, "member_runs.status", "new input requires an active running member"); err != nil {
			return inputReservation{}, err
		}
		if err := requireCapacity(ctx, conn, run.runID); err != nil {
			return inputReservation{}, err
		}
		now := nowStamp()
		selection, selected := reservation.selection.get()
		deferReason, deferred := reservation.deferReason.get()
		if err := brokerRuntimeRequire(!selected || !deferred, "delivery_method", "a queued input selects its method when its attempt starts"); err != nil {
			return inputReservation{}, err
		}
		state := "in_progress"
		if deferred {
			state = "ready"
		}
		method, reason := none[string](), none[string]()
		if selected {
			method, reason = some(selection.method.asStr()), selection.reason
		}
		deferredAt, deferredReason := none[string](), none[string]()
		if deferred {
			deferredAt, deferredReason = some(now), some(deferReason)
		}
		if _, err := conn.execute(
			`INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at,delivery_method,delivery_reason,deferred_at,defer_reason)
             VALUES(?1,?2,?3,?4,?5,?6,?6,?7,?8,?9,?10)`,
			run.runID, request.operationKey, request.kind.asStr(), request.payloadDigest,
			state, now, method, reason, deferredAt, deferredReason); err != nil {
			return inputReservation{}, err
		}
		inserted, err := getInput(ctx, conn, conn.raw.LastInsertRowID())
		if err != nil {
			return inputReservation{}, err
		}
		return inputReservation{operation: inserted, inserted: true}, nil
	})
}

func ownedInput(ctx context.Context, conn *dbConn, run runRecord, inputID int64) (inputOperation, error) {
	input, err := getInput(ctx, conn, inputID)
	if err != nil {
		return inputOperation{}, err
	}
	if err := brokerRuntimeRequire(input.runID == run.runID, "input_id", "input belongs to another run"); err != nil {
		return inputOperation{}, err
	}
	return input, nil
}

func recordInputState(ctx context.Context, conn *dbConn, owner runOwner, inputID int64, expected, next inputState) (inputOperation, error) {
	valid := expected == inputStateInProgress && (next == inputStateReady || next == inputStateAccepted)
	if err := brokerRuntimeRequire(valid, "member_input_operations.state", "invalid input transition"); err != nil {
		return inputOperation{}, err
	}
	return ownedTransaction(ctx, conn, owner, func(ctx context.Context, conn *dbConn, run runRecord) (inputOperation, error) {
		input, err := ownedInput(ctx, conn, run, inputID)
		if err != nil {
			return inputOperation{}, err
		}
		if err := brokerRuntimeRequire(input.state == expected, "member_input_operations.state", "input state changed"); err != nil {
			return inputOperation{}, err
		}
		sql := "UPDATE member_input_operations SET state=?1,updated_at=?2 WHERE input_id=?3 AND state=?4"
		if next == inputStateReady {
			sql = "UPDATE member_input_operations SET state=?1,updated_at=?2,delivery_method=NULL,delivery_reason=NULL WHERE input_id=?3 AND state=?4"
		}
		return updateInput(ctx, conn, inputID, sql, []any{next.asStr(), nowStamp(), inputID, expected.asStr()}, casFailed)
	})
}

const casFailed = "input CAS failed"

// updateInput applies one compare-and-set UPDATE to an input row and returns the updated input.
func updateInput(ctx context.Context, conn *dbConn, inputID int64, sql string, args []any, failure string) (inputOperation, error) {
	changed, err := conn.execute(sql, args...)
	if err != nil {
		return inputOperation{}, err
	}
	if err := brokerRuntimeRequire(changed == 1, "member_input_operations.state", failure); err != nil {
		return inputOperation{}, err
	}
	return getInput(ctx, conn, inputID)
}

func recordInputUncertain(ctx context.Context, conn *dbConn, owner runOwner, inputID int64, cause stopCause) (inputOperation, error) {
	return ownedTransaction(ctx, conn, owner, func(ctx context.Context, conn *dbConn, run runRecord) (inputOperation, error) {
		input, err := ownedInput(ctx, conn, run, inputID)
		if err != nil {
			return inputOperation{}, err
		}
		if err := brokerRuntimeRequire(input.state == inputStateInProgress, "member_input_operations.state", "input state changed"); err != nil {
			return inputOperation{}, err
		}
		return updateInput(ctx, conn, inputID,
			"UPDATE member_input_operations SET state='uncertain',updated_at=?1,stop_cause=?2 WHERE input_id=?3 AND state='in_progress'",
			[]any{nowStamp(), cause.asStr(), inputID}, casFailed)
	})
}

// settleInputInTransaction settles inputID of run, whose ownership the caller's transaction has
// validated.
func settleInputInTransaction(ctx context.Context, conn *dbConn, run runRecord, inputID int64, expected inputState, disposition inputDisposition, evidenceRef string) (inputOperation, error) {
	if err := brokerRuntimeNonblank(evidenceRef, "evidence_ref"); err != nil {
		return inputOperation{}, err
	}
	if err := brokerRuntimeRequire(expected != inputStateSettled, "state", "settled operations retain their disposition"); err != nil {
		return inputOperation{}, err
	}
	input, err := ownedInput(ctx, conn, run, inputID)
	if err != nil {
		return inputOperation{}, err
	}
	if err := brokerRuntimeRequire(input.state == expected, "member_input_operations.state", "input state changed"); err != nil {
		return inputOperation{}, err
	}
	return settle(ctx, conn, inputID, expected, disposition, some(evidenceRef))
}

func settle(ctx context.Context, conn *dbConn, inputID int64, expected inputState, disposition inputDisposition, evidenceRef opt[string]) (inputOperation, error) {
	return updateInput(ctx, conn, inputID,
		`UPDATE member_input_operations SET state='settled',disposition=?1,evidence_ref=?2,updated_at=?3
         WHERE input_id=?4 AND state=?5`,
		[]any{disposition.asStr(), evidenceRef, nowStamp(), inputID, expected.asStr()}, "settlement CAS failed")
}

// stampPreview stamps the message of a preview the run's sweep delivered, under the run's
// ownership; an already stamped message stays unchanged.
func stampPreview(ctx context.Context, conn *dbConn, owner runOwner, messageID int64) error {
	_, err := ownedTransaction(ctx, conn, owner, func(ctx context.Context, conn *dbConn, run runRecord) (struct{}, error) {
		message, err := getMessage(ctx, conn, messageID)
		if err != nil {
			return struct{}{}, err
		}
		if err := brokerRuntimeRequire(message.toMemberID == run.memberID, "message_id", "preview recipient changed"); err != nil {
			return struct{}{}, err
		}
		_, err = conn.execute("UPDATE messages SET notified_at=?1 WHERE message_id=?2 AND notified_at IS NULL", nowStamp(), messageID)
		return struct{}{}, err
	})
	return err
}

// stampDirectorPreview stamps a Director preview's notified_at; false when it was already set.
func stampDirectorPreview(ctx context.Context, conn *dbConn, owner runOwner, messageID int64) (bool, error) {
	return ownedTransaction(ctx, conn, owner, func(ctx context.Context, conn *dbConn, run runRecord) (bool, error) {
		message, err := getMessage(ctx, conn, messageID)
		if err != nil {
			return false, err
		}
		if err := brokerRuntimeRequire(message.toMemberID == run.directorMemberID, "message_id", "director preview recipient changed"); err != nil {
			return false, err
		}
		stamped, err := conn.execute("UPDATE messages SET notified_at=?1 WHERE message_id=?2 AND notified_at IS NULL", nowStamp(), messageID)
		if err != nil {
			return false, err
		}
		return stamped == 1, nil
	})
}

const unresolvedInput = "(state IN ('in_progress','accepted','uncertain') OR (state='ready' AND deferred_at IS NOT NULL))"

func requireCapacity(_ context.Context, conn *dbConn, runID int64) error {
	unresolved, err := queryRow(conn,
		"SELECT COUNT(*) FROM member_input_operations WHERE run_id=?1 AND "+unresolvedInput,
		[]any{runID}, func(row sqlRow) (int64, error) { return row.int64(0) })
	if err != nil {
		return err
	}
	return brokerRuntimeRequire(unresolved < 4096, "member_input_operations", "unresolved input limit reached")
}

// settleOwnedInputInTransaction settles expected at its current state inside the caller's
// ownership transaction.
func settleOwnedInputInTransaction(ctx context.Context, conn *dbConn, owner runOwner, expected inputOperation, disposition inputDisposition, evidenceRef string) (inputOperation, error) {
	if err := brokerRuntimeRequire(!conn.isAutocommit(), "transaction", "input settlement requires an active ownership transaction"); err != nil {
		return inputOperation{}, err
	}
	run, err := validateOwner(ctx, conn, owner)
	if err != nil {
		return inputOperation{}, err
	}
	return settleInputInTransaction(ctx, conn, run, expected.inputID, expected.state, disposition, evidenceRef)
}
