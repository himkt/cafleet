package core

import (
	"context"
	"fmt"
	"slices"
	"strconv"
	"strings"
	"testing"
)

var newReservationPlain = newReservation{
	selection:   some(reservationSelection{method: deliveryMethodProfile, reason: none[string]()}),
	deferReason: none[string](),
}

func mustInput(t *testing.T) func(inputOperation, error) inputOperation {
	t.Helper()
	return func(operation inputOperation, err error) inputOperation {
		t.Helper()
		if err != nil {
			t.Fatal(err)
		}
		return operation
	}
}

func TestBrokerRuntimeInputs_InputPayloadDigestHashesTheKeySortedPayload(t *testing.T) {
	t.Parallel()
	const literal = `say "hi" λ`
	quoted := mustEncodeJSON(t, literal)
	expected := func(kind string) string {
		return sha256Hex([]byte(`{"kind":"` + kind + `","literal":` + quoted + `,"run_id":7}`))
	}
	cases := []struct {
		name  string
		input submitInput
		want  string
	}{
		{name: "prompt", input: submitInput{OperationKey: "prompt:7", InputKind: inputKindPrompt, Literal: literal}, want: expected("prompt")},
		{name: "exec", input: submitInput{OperationKey: "exec:7", InputKind: inputKindExec, Literal: literal}, want: expected("exec")},
	}
	for _, tc := range cases {
		if got := inputPayloadDigest(7, tc.input); got != tc.want {
			t.Fatalf("%s: inputPayloadDigest = %s, want %s", tc.name, got, tc.want)
		}
	}
}

func TestBrokerRuntimeInputs_ReceiptKeyDigestAndSettledDispositionSurviveReconnection(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newBrokerRuntimeFixture(t)
	owner := fixture.activate(t)
	input := fixture.input(t, owner, "prompt:one", inputKindPrompt)
	if input.state != inputStateInProgress {
		t.Fatalf("a reserved receipt is in progress, got %v", input.state)
	}
	request := newInput{operationKey: "prompt:one", kind: inputKindPrompt, payloadDigest: strings.Repeat("d", 64)}
	other := fixture.reopen(t)
	if again := mustInput(t)(reserveInput(ctx, other, owner, request)); again.inputID != input.inputID {
		t.Fatalf("a reconnected reservation = %d, want %d", again.inputID, input.inputID)
	}
	settled := mustInput(t)(settleInput(ctx, other, owner, input.inputID, inputStateInProgress, inputDispositionNotSubmitted, "audit:zero-byte-confirmation"))
	if settled.state != inputStateSettled {
		t.Fatalf("settled state = %v", settled.state)
	}
	retry := mustInput(t)(reserveInput(ctx, fixture.conn, owner, request))
	if retry.inputID != input.inputID || retry.disposition != some(inputDispositionNotSubmitted) {
		t.Fatalf("retry = %+v", retry)
	}
	if rows := runtimeTableRows(t, fixture.conn, "member_input_operations"); len(rows) != 1 {
		t.Fatalf("inputs = %d", len(rows))
	}
}

func TestBrokerRuntimeInputs_OperationKeyRejectsAChangedDigestOrKind(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newBrokerRuntimeFixture(t)
	owner := fixture.activate(t)
	fixture.input(t, owner, "same-key", inputKindPrompt)
	for _, changed := range []string{"digest", "kind"} {
		request := newInput{operationKey: "same-key", kind: inputKindPrompt, payloadDigest: strings.Repeat("d", 64)}
		switch changed {
		case "digest":
			request.payloadDigest = strings.Repeat("e", 64)
		case "kind":
			request.kind = inputKindExec
		}
		before := durableState(t, fixture.conn)
		if _, err := reserveInput(ctx, fixture.conn, owner, request); err == nil {
			t.Fatalf("%s: a conflicting key reuse is refused", changed)
		}
		if !durableStatesEqual(durableState(t, fixture.conn), before) {
			t.Fatalf("%s: the refusal changed durable state", changed)
		}
	}
}

func TestBrokerRuntimeInputs_SettlementCasRejectsStaleStateAndPreservesRecordedEvidence(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newBrokerRuntimeFixture(t)
	owner := fixture.activate(t)
	input := fixture.input(t, owner, "exec:lost-stdout", inputKindExec)
	mustInput(t)(recordInputUncertain(ctx, fixture.conn, owner, input.inputID, stopCauseWriteFailed))
	if _, err := settleInput(ctx, fixture.conn, owner, input.inputID, inputStateInProgress, inputDispositionSubmitted, "audit:stale"); err == nil {
		t.Fatal("a stale expected state is refused")
	}
	if _, err := settleInput(ctx, fixture.conn, owner, input.inputID, inputStateUncertain, inputDispositionSubmitted, " "); err == nil {
		t.Fatal("a blank evidence reference is refused")
	}
	settled := mustInput(t)(settleInput(ctx, fixture.conn, owner, input.inputID, inputStateUncertain, inputDispositionAbandonedUncertain, "audit:inspected-output"))
	if settled.disposition != some(inputDispositionAbandonedUncertain) || settled.evidenceRef != some("audit:inspected-output") {
		t.Fatalf("settled = %+v", settled)
	}
	if read := mustInput(t)(getInput(ctx, fixture.reopen(t), input.inputID)); read.state != inputStateSettled {
		t.Fatalf("reread state = %v", read.state)
	}
	if _, err := settleInput(ctx, fixture.conn, owner, input.inputID, inputStateUncertain, inputDispositionNotSubmitted, "audit:changed"); err == nil {
		t.Fatal("a settled receipt keeps its disposition")
	}
	if rows := runtimeTableRows(t, fixture.conn, "member_runtime_events"); len(rows) != 0 {
		t.Fatalf("settlement wrote events: %v", rows)
	}
}

func TestBrokerRuntimeInputs_ReconcilingAnUncertainReceiptWithARecordedMethodNotSubmittedReadsBack(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newBrokerRuntimeFixture(t)
	owner := fixture.activate(t)
	input := fixture.input(t, owner, "prompt:uncertain", inputKindPrompt)
	mustInput(t)(requeueZeroByteImmediate(ctx, fixture.conn, owner, input.inputID, "interaction_open"))
	mustInput(t)(beginDeferredAttempt(ctx, fixture.conn, owner, input.inputID, deliveryMethodFallback, some("submission_plan_unavailable")))
	mustInput(t)(recordInputUncertain(ctx, fixture.conn, owner, input.inputID, stopCauseShortWrite))
	settled := mustInput(t)(settleInput(ctx, fixture.conn, owner, input.inputID, inputStateUncertain, inputDispositionNotSubmitted, "audit:no-bytes-observed"))
	read := mustInput(t)(getInput(ctx, fixture.reopen(t), input.inputID))
	if read != settled {
		t.Fatalf("reread %+v, want %+v", read, settled)
	}
	if read.state != inputStateSettled || read.disposition != some(inputDispositionNotSubmitted) ||
		read.deliveryMethod != some(deliveryMethodFallback) || read.deliveryReason != some("submission_plan_unavailable") {
		t.Fatalf("read = %+v", read)
	}
	if status := read.delivery().Status; status != deliveryStatusNotSubmitted {
		t.Fatalf("delivery status = %v", status)
	}
}

func TestBrokerRuntimeInputs_SettledReceiptHistoryCanExceedTheUnresolvedCacheBound(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newBrokerRuntimeFixture(t)
	owner := fixture.activate(t)
	if _, err := fixture.conn.execute(`WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<4095)
		INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at,disposition,evidence_ref)
		SELECT ?1,'settled:'||x,'prompt',?2,'settled',?3,?3,'not_submitted','audit:zero-byte' FROM n`,
		owner.runID, strings.Repeat("d", 64), runtimeFixtureNow); err != nil {
		t.Fatal(err)
	}
	input := fixture.input(t, owner, "settled:4096", inputKindPrompt)
	mustInput(t)(settleInput(ctx, fixture.conn, owner, input.inputID, inputStateInProgress, inputDispositionNotSubmitted, "audit:zero-byte"))
	if rows := runtimeTableRows(t, fixture.conn, "member_input_operations"); len(rows) != 4097 {
		t.Fatalf("inputs = %d", len(rows))
	}
	found, err := findInput(ctx, fixture.reopen(t), owner.runID, "settled:0")
	if _, ok := found.get(); err != nil || !ok {
		t.Fatalf("findInput(settled:0) = %v %v", found, err)
	}
}

func TestBrokerRuntimeInputs_FailedInputReservationAndCorruptReceiptReadsSurfaceTheOriginalState(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newBrokerRuntimeFixture(t)
	owner := fixture.activate(t)
	fault := faultPlanAbort("fail_input", "BEFORE INSERT ON member_input_operations", "injected input failure")
	fault.arm(t, fixture.conn)
	if _, err := reserveInput(ctx, fixture.conn, owner, newInput{operationKey: "reserved", kind: inputKindExec, payloadDigest: strings.Repeat("d", 64)}); err == nil {
		t.Fatal("the injected failure refuses the reservation")
	}
	if rows := runtimeTableRows(t, fixture.reopen(t), "member_input_operations"); len(rows) != 0 {
		t.Fatalf("the failed reservation left rows: %v", rows)
	}
	fault.clear(t, fixture.conn)
	input := fixture.input(t, owner, "reserved", inputKindExec)
	mustExecBatch(t, fixture.conn, "PRAGMA ignore_check_constraints=ON")
	if _, err := fixture.conn.execute("UPDATE member_input_operations SET state='corrupt' WHERE input_id=?1", input.inputID); err != nil {
		t.Fatal(err)
	}
	_, err := getInput(ctx, fixture.conn, input.inputID)
	if err == nil || !strings.Contains(err.Error(), "state") || !strings.Contains(err.Error(), "corrupt") {
		t.Fatalf("getInput = %v", err)
	}
}

func testDeferredAttempt(t *testing.T, fixture *brokerRuntimeFixture, owner runOwner, key string) inputOperation {
	t.Helper()
	ctx := context.Background()
	input := fixture.input(t, owner, key, inputKindPrompt)
	mustInput(t)(requeueZeroByteImmediate(ctx, fixture.conn, owner, input.inputID, "interaction_open"))
	return mustInput(t)(beginDeferredAttempt(ctx, fixture.conn, owner, input.inputID, deliveryMethodProfile, none[string]()))
}

func TestBrokerRuntimeInputs_StopCause_RecordInputUncertainPersistsEachCauseAcrossReconnection(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newBrokerRuntimeFixture(t)
	owner := fixture.activate(t)
	causes := []stopCause{stopCauseInteractionCandidate, stopCauseInteractionOpen, stopCauseDeadlineExpired, stopCauseShortWrite, stopCauseWriteFailed, stopCauseRunnerStopped}
	for index, cause := range causes {
		key := fmt.Sprintf("exec:cause:%d", index)
		input := fixture.input(t, owner, key, inputKindExec)
		if input.stopCause != none[stopCause]() {
			t.Fatalf("%v: a fresh receipt has no cause", cause)
		}
		uncertain, err := recordInputUncertain(ctx, fixture.conn, owner, input.inputID, cause)
		if err != nil {
			t.Fatalf("%v: %v", cause, err)
		}
		if uncertain.state != inputStateUncertain || uncertain.stopCause != some(cause) || uncertain.delivery().Status != deliveryStatusUncertain {
			t.Fatalf("%v: uncertain = %+v", cause, uncertain)
		}
		if read := mustInput(t)(getInput(ctx, fixture.reopen(t), input.inputID)); read != uncertain {
			t.Fatalf("%v: reread %+v", cause, read)
		}
		found, err := findInput(ctx, fixture.reopen(t), owner.runID, key)
		if record, ok := found.get(); err != nil || !ok || record.stopCause != some(cause) {
			t.Fatalf("%v: findInput = %+v %v", cause, found, err)
		}
	}
}

func TestBrokerRuntimeInputs_StopCause_AnUncertainTransitionRequiresAnOwnedInProgressReceiptAndChangesNothing(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name    string
		message string
	}{
		{"plain state transition", "invalid input transition"},
		{"ready", "input state changed"},
		{"accepted", "input state changed"},
		{"already uncertain", "input state changed"},
		{"superseded owner", ""},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			ctx := context.Background()
			fixture := newBrokerRuntimeFixture(t)
			owner := fixture.activate(t)
			input := fixture.input(t, owner, "prompt:refused", inputKindPrompt)
			switch tc.name {
			case "ready":
				mustInput(t)(requeueZeroByteImmediate(ctx, fixture.conn, owner, input.inputID, "interaction_open"))
			case "accepted":
				mustInput(t)(recordInputState(ctx, fixture.conn, owner, input.inputID, inputStateInProgress, inputStateAccepted))
			case "already uncertain":
				mustInput(t)(recordInputUncertain(ctx, fixture.conn, owner, input.inputID, stopCauseWriteFailed))
			case "superseded owner":
				fixture.activateNextEpoch(t)
			}
			before := durableState(t, fixture.conn)
			var err error
			if tc.name == "plain state transition" {
				_, err = recordInputState(ctx, fixture.conn, owner, input.inputID, inputStateInProgress, inputStateUncertain)
			} else {
				_, err = recordInputUncertain(ctx, fixture.conn, owner, input.inputID, stopCauseShortWrite)
			}
			if err == nil || !strings.Contains(err.Error(), tc.message) {
				t.Fatalf("%s: error %v, want one containing %q", tc.name, err, tc.message)
			}
			if !durableStatesEqual(durableState(t, fixture.conn), before) {
				t.Fatalf("%s: the refusal changed durable state", tc.name)
			}
			stored := mustInput(t)(getInput(ctx, fixture.conn, input.inputID))
			switch tc.name {
			case "plain state transition":
				if stored.state != inputStateInProgress {
					t.Fatalf("state = %v", stored.state)
				}
			case "already uncertain":
				if stored.stopCause != some(stopCauseWriteFailed) {
					t.Fatalf("a repeated transition keeps the first recorded cause, got %v", stored.stopCause)
				}
			}
		})
	}
}

func TestBrokerRuntimeInputs_StopCause_ImmediateStoppedResultsRemainUnpublishedUntilTheirFirstDurableEvent(t *testing.T) {
	t.Parallel()
	for _, kind := range []inputKind{inputKindPrompt, inputKindExec} {
		t.Run(kind.asStr(), func(t *testing.T) {
			t.Parallel()
			ctx := context.Background()
			fixture := newBrokerRuntimeFixture(t)
			owner := fixture.activate(t)
			uncertain := fixture.input(t, owner, "ordinary:immediate-uncertain", kind)
			mustInput(t)(recordInputUncertain(ctx, fixture.conn, owner, uncertain.inputID, stopCauseWriteFailed))
			submitted := fixture.input(t, owner, "ordinary:immediate-submitted", kind)
			mustInput(t)(recordInputState(ctx, fixture.conn, owner, submitted.inputID, inputStateInProgress, inputStateAccepted))
			expected := []unpublishedDeliveryResult{{inputID: uncertain.inputID, stage: "unknown", stopCause: some(stopCauseWriteFailed)}}
			assertUnpublished(t, fixture, owner.runID, expected)
			for index, result := range slices.Clone(expected) {
				transition := observedTransition{
					RunID:        owner.runID,
					EventSeq:     int64(index) + 1,
					EpisodeSeq:   none[int64](),
					Kind:         runtimeEventKindInputDeliveryResult,
					ObservedAt:   runtimeFixtureNow,
					EvidenceJSON: runtimeMetadata(t),
					CauseJSON:    inputDeliveryResultCause(result.inputID, result.stage, result.stopCause),
				}
				event, inserted, err := persistTransition(ctx, fixture.conn, owner, transition, persistenceModeLive)
				if err != nil || !inserted {
					t.Fatalf("persistTransition = %v %v", inserted, err)
				}
				view, err := getEvent(ctx, fixture.conn, event.eventID)
				if err != nil {
					t.Fatal(err)
				}
				assertJSONEqual(t, jsonAt(t, view.event.payloadJSON, "input_id"), mustJSON(t, result.inputID), "the published input")
				expected = expected[1:]
				assertUnpublished(t, fixture, owner.runID, expected)
			}
		})
	}
}

func assertUnpublished(t *testing.T, fixture *brokerRuntimeFixture, runID int64, want []unpublishedDeliveryResult) {
	t.Helper()
	got, err := unpublishedDeliveryResults(context.Background(), fixture.conn, runID)
	if err != nil || !slices.Equal(got, want) {
		t.Fatalf("unpublishedDeliveryResults = %+v %v, want %+v", got, err, want)
	}
}

func TestBrokerRuntimeInputs_StopCause_UnpublishedDeliveryResultsReturnTheReceiptCause(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newBrokerRuntimeFixture(t)
	owner := fixture.activate(t)
	uncertain := testDeferredAttempt(t, fixture, owner, "prompt:deferred-uncertain")
	mustInput(t)(recordInputUncertain(ctx, fixture.conn, owner, uncertain.inputID, stopCauseWriteFailed))
	submitted := testDeferredAttempt(t, fixture, owner, "prompt:deferred-submitted")
	mustInput(t)(recordInputState(ctx, fixture.conn, owner, submitted.inputID, inputStateInProgress, inputStateAccepted))
	abandoned := testDeferredAttempt(t, fixture, owner, "prompt:deferred-abandoned")
	mustInput(t)(recordInputUncertain(ctx, fixture.conn, owner, abandoned.inputID, stopCauseRunnerStopped))
	mustInput(t)(settleInput(ctx, fixture.conn, owner, abandoned.inputID, inputStateUncertain, inputDispositionAbandonedUncertain, "audit:abandoned"))
	expected := []unpublishedDeliveryResult{
		{inputID: uncertain.inputID, stage: "unknown", stopCause: some(stopCauseWriteFailed)},
		{inputID: submitted.inputID, stage: "submitted", stopCause: none[stopCause]()},
		{inputID: abandoned.inputID, stage: "unknown", stopCause: some(stopCauseRunnerStopped)},
	}
	for _, reconciled := range []struct {
		key         string
		disposition inputDisposition
		stage       string
	}{
		{"prompt:reconciled-submitted", inputDispositionSubmitted, "submitted"},
		{"prompt:reconciled-not-submitted", inputDispositionNotSubmitted, "zero_bytes"},
	} {
		input := testDeferredAttempt(t, fixture, owner, reconciled.key)
		mustInput(t)(recordInputUncertain(ctx, fixture.conn, owner, input.inputID, stopCauseWriteFailed))
		settled := mustInput(t)(settleInput(ctx, fixture.conn, owner, input.inputID, inputStateUncertain, reconciled.disposition, "audit:inspected"))
		if settled.stopCause != some(stopCauseWriteFailed) {
			t.Fatalf("%s: settled cause = %v", reconciled.key, settled.stopCause)
		}
		expected = append(expected, unpublishedDeliveryResult{inputID: input.inputID, stage: reconciled.stage, stopCause: none[stopCause]()})
	}
	lost := testDeferredAttempt(t, fixture, owner, "prompt:deferred-lost")
	expected = append(expected, unpublishedDeliveryResult{inputID: lost.inputID, stage: "unknown", stopCause: none[stopCause]()})
	assertUnpublished(t, fixture, owner.runID, expected)
}

func TestBrokerRuntimeInputs_InputPresentationProjectsDeliveryFromTheStoredReceiptState(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	const now = "2026-10-30T10:00:00.000000+00:00"
	fixture := newBrokerRuntimeFixture(t)
	owner := fixture.activate(t)
	cases := []struct {
		assignment string
		delivery   string
		deferred   bool
	}{
		{"state='ready',delivery_method=NULL,deferred_at='" + now + "',defer_reason='interaction_open'", `{"status":"queued","method":null,"reason":"interaction_open"}`, true},
		{"state='ready',delivery_method=NULL", `{"status":"held","method":null,"reason":"zero_bytes_ready"}`, false},
		{"delivery_method='fallback',delivery_reason='submission_plan_unavailable'", `{"status":"uncertain","method":"fallback","reason":"submission_plan_unavailable"}`, false},
		{"state='accepted',delivery_method='profile'", `{"status":"submitted","method":"profile","reason":null}`, false},
		{"state='settled',disposition='submitted',delivery_method='fallback',delivery_reason='submission_plan_unavailable'", `{"status":"submitted","method":"fallback","reason":"submission_plan_unavailable"}`, false},
		{"state='settled',disposition='not_submitted',delivery_method=NULL,delivery_reason='runner_stopped'", `{"status":"not_submitted","method":null,"reason":"runner_stopped"}`, false},
		{"state='settled',disposition='not_submitted',delivery_method='profile'", `{"status":"not_submitted","method":"profile","reason":null}`, false},
		{"state='settled',disposition='abandoned_uncertain',delivery_method='profile'", `{"status":"uncertain","method":"profile","reason":null}`, false},
	}
	for index, tc := range cases {
		input := fixture.input(t, owner, "prompt:"+strconv.Itoa(index), inputKindPrompt)
		if _, err := fixture.conn.execute("UPDATE member_input_operations SET "+tc.assignment+" WHERE input_id=?1", input.inputID); err != nil {
			t.Fatalf("%s: %v", tc.assignment, err)
		}
		stored, err := getInput(ctx, fixture.conn, input.inputID)
		if err != nil {
			t.Fatalf("%s: %v", tc.assignment, err)
		}
		if got := mustEncodeJSON(t, stored.delivery().toJSON()); got != tc.delivery {
			t.Fatalf("%s: delivery = %s, want %s", tc.assignment, got, tc.delivery)
		}
		_, deferredAt := stored.deferredAt.get()
		_, deferReason := stored.deferReason.get()
		if deferredAt != tc.deferred || deferReason != tc.deferred {
			t.Fatalf("%s: deferred = %v/%v, want %v", tc.assignment, deferredAt, deferReason, tc.deferred)
		}
	}
	queuedWithMethod := fixture.input(t, owner, "exec:invalid", inputKindExec)
	if _, err := fixture.conn.execute("UPDATE member_input_operations SET state='ready',deferred_at='"+now+"',defer_reason='interaction_open',delivery_method='profile' WHERE input_id=?1", queuedWithMethod.inputID); err != nil {
		t.Fatal(err)
	}
	if _, err := getInput(ctx, fixture.conn, queuedWithMethod.inputID); err == nil {
		t.Fatal("a queued receipt with a selected method fails validation")
	}
}

func TestBrokerRuntimeInputs_FirstUncertainOrdinaryInputReturnsTheRunsLowestUncertainPromptOrExec(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newBrokerRuntimeFixture(t)
	owner := fixture.activate(t)
	first := func(runID int64) opt[int64] {
		t.Helper()
		found, err := firstUncertainOrdinaryInput(ctx, fixture.conn, runID)
		if err != nil {
			t.Fatal(err)
		}
		return found
	}
	if got := first(owner.runID); got != none[int64]() {
		t.Fatalf("a run without input has no blocker, got %v", got)
	}
	ready := fixture.input(t, owner, "prompt:ready", inputKindPrompt)
	mustInput(t)(recordInputState(ctx, fixture.conn, owner, ready.inputID, inputStateInProgress, inputStateReady))
	settled := fixture.input(t, owner, "exec:settled", inputKindExec)
	mustInput(t)(recordInputUncertain(ctx, fixture.conn, owner, settled.inputID, stopCauseWriteFailed))
	mustInput(t)(settleInput(ctx, fixture.conn, owner, settled.inputID, inputStateUncertain, inputDispositionNotSubmitted, "audit:pane-shows-no-input"))
	if got := first(owner.runID); got != none[int64]() {
		t.Fatalf("ready and settled inputs are no blocker, got %v", got)
	}
	var uncertain []int64
	for _, entry := range []struct {
		key  string
		kind inputKind
	}{{"exec:first-uncertain", inputKindExec}, {"prompt:second-uncertain", inputKindPrompt}} {
		input := fixture.input(t, owner, entry.key, entry.kind)
		mustInput(t)(recordInputUncertain(ctx, fixture.conn, owner, input.inputID, stopCauseWriteFailed))
		uncertain = append(uncertain, input.inputID)
	}
	if got := first(owner.runID); got != some(uncertain[0]) {
		t.Fatalf("first blocker = %v, want %d", got, uncertain[0])
	}
	mustInput(t)(settleInput(ctx, fixture.conn, owner, uncertain[0], inputStateUncertain, inputDispositionSubmitted, "audit:pane-shows-the-input"))
	if got := first(owner.runID); got != some(uncertain[1]) {
		t.Fatalf("settling the first blocker exposes the next, got %v", got)
	}
	if got := first(owner.runID + 1000); got != none[int64]() {
		t.Fatalf("another run's inputs are never its blocker, got %v", got)
	}
}

func messageNotified(t *testing.T, fixture *brokerRuntimeFixture, messageID int64) opt[string] {
	t.Helper()
	return messageNotifiedAt(t, fixture.conn, messageID)
}

func TestBrokerRuntimeInputs_DirectorPreviewStamp_ADirectorPreviewStampSucceedsOnlyWhileNotifiedAtIsNull(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newBrokerRuntimeFixture(t)
	owner := fixture.activate(t)
	message := mustSend(t, fixture.conn, fixture.member, fixture.director, "report for the Director").messageID
	if got := messageNotified(t, fixture, message); got != none[string]() {
		t.Fatalf("notified_at = %v", got)
	}
	if stamped, err := stampDirectorPreview(ctx, fixture.conn, owner, message); err != nil || !stamped {
		t.Fatalf("the first stamp = %v %v", stamped, err)
	}
	stamped := messageNotified(t, fixture, message)
	if _, ok := stamped.get(); !ok {
		t.Fatal("the first stamp records notified_at")
	}
	if again, err := stampDirectorPreview(ctx, fixture.conn, owner, message); err != nil || again {
		t.Fatalf("a second stamp finds the column set: %v %v", again, err)
	}
	if got := messageNotified(t, fixture, message); got != stamped {
		t.Fatalf("notified_at changed to %v", got)
	}
}

func TestBrokerRuntimeInputs_DirectorPreviewStamp_ADirectorPreviewStampRefusesAMessageToAnotherRecipient(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newBrokerRuntimeFixture(t)
	owner := fixture.activate(t)
	message := mustSend(t, fixture.conn, fixture.director, fixture.member, "assignment for the member").messageID
	if _, err := stampDirectorPreview(ctx, fixture.conn, owner, message); err == nil {
		t.Fatal("a message to another recipient is refused")
	}
	if got := messageNotified(t, fixture, message); got != none[string]() {
		t.Fatalf("notified_at = %v", got)
	}
}

func TestBrokerRuntimeInputs_DirectorPreviewStamp_ADirectorPreviewStampRequiresTheCurrentRunOwner(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newBrokerRuntimeFixture(t)
	stale := fixture.activate(t)
	fixture.activateNextEpoch(t)
	message := mustSend(t, fixture.conn, fixture.member, fixture.director, "report after an ownership change").messageID
	if _, err := stampDirectorPreview(ctx, fixture.conn, stale, message); err == nil {
		t.Fatal("a stale owner is refused")
	}
	if got := messageNotified(t, fixture, message); got != none[string]() {
		t.Fatalf("notified_at = %v", got)
	}
}
